pub use embedded_svc::utils::io as utils;

pub use esp_idf_hal::io::*;

#[path = "ioasync.rs"]
mod ioasync;

#[cfg(esp_idf_comp_vfs_enabled)]
pub mod vfs {
    use crate::sys;

    pub use super::ioasync::*;

    #[allow(clippy::needless_update)]
    pub fn initialize_eventfd(max_fds: usize) -> Result<(), sys::EspError> {
        sys::esp!(unsafe {
            sys::esp_vfs_eventfd_register(&sys::esp_vfs_eventfd_config_t {
                max_fds: max_fds as _,
                ..Default::default()
            })
        })
    }
}

pub mod reactor {
    use core::ffi::{c_void, CStr};
    use core::sync::atomic::{AtomicPtr, Ordering};
    use core::task::Waker;

    use std::io::{self, ErrorKind};
    use std::os::fd::RawFd;

    use enumset::{EnumSet, EnumSetType};

    use log::info;

    use crate::hal::cpu::Core;
    use crate::hal::task::CriticalSection;
    use crate::sys::esp;
    use crate::sys::fd_set;

    const MAX_REGISTRATIONS: usize = 20;

    const FD_SEGMENT: usize = 32;

    const FD_ZERO: fd_set = fd_set {
        __fds_bits: [0; crate::sys::MAX_FDS as usize / FD_SEGMENT],
    };

    #[derive(EnumSetType, Debug)]
    pub(crate) enum Event {
        Read = 0,
        Write = 1,
    }

    struct Fds {
        read: fd_set,
        write: fd_set,
        except: fd_set,
    }

    impl Fds {
        const fn new() -> Self {
            Self {
                read: FD_ZERO,
                write: FD_ZERO,
                except: FD_ZERO,
            }
        }

        fn is_set(&self, fd: RawFd, event: Event) -> bool {
            self.fd_set(event).__fds_bits[fd as usize / FD_SEGMENT]
                & (1 << (fd as usize % FD_SEGMENT))
                != 0
        }

        fn set(&mut self, fd: RawFd, event: Event) {
            self.fd_set(event).__fds_bits[fd as usize / FD_SEGMENT] |=
                1 << (fd as usize % FD_SEGMENT);
        }

        fn fd_set(&mut self, event: Event) -> &mut fd_set {
            match event {
                Event::Read => &mut self.read,
                Event::Write => &mut self.write,
            }
        }
    }

    struct Registration {
        fd: RawFd,
        events: EnumSet<Event>,
        wakers: [Option<Waker>; 2],
    }

    struct Registrations<const N: usize> {
        vec: heapless::Vec<Registration, N>,
        event_fd: Option<RawFd>,
    }

    impl<const N: usize> Registrations<N> {
        const fn new() -> Self {
            Self {
                vec: heapless::Vec::new(),
                event_fd: None,
            }
        }

        fn register(&mut self, fd: RawFd) -> io::Result<()> {
            if self.vec.iter().any(|reg| reg.fd == fd) {
                Err(ErrorKind::InvalidInput)?;
            }

            self.vec
                .push(Registration {
                    fd,
                    events: EnumSet::empty(),
                    wakers: [None, None],
                })
                .map_err(|_| ErrorKind::OutOfMemory)?;

            Ok(())
        }

        fn deregister(&mut self, fd: RawFd) -> io::Result<()> {
            let Some(index) = self.vec.iter_mut().position(|reg| reg.fd == fd) else {
                return Err(ErrorKind::NotFound.into());
            };

            self.vec.swap_remove(index);

            self.notify()?;

            Ok(())
        }

        fn set(&mut self, fd: RawFd, event: Event, waker: &Waker) -> io::Result<()> {
            let Some(mut registration) = self.vec.iter_mut().find(|reg| reg.fd == fd) else {
                return Err(ErrorKind::NotFound.into());
            };

            registration.events.remove(event);

            if let Some(prev_waker) = registration.wakers[event as usize].replace(waker.clone()) {
                if !prev_waker.will_wake(waker) {
                    prev_waker.wake();
                }
            }

            self.notify()?;

            Ok(())
        }

        fn fetch(&mut self, fd: RawFd, event: Event) -> io::Result<bool> {
            let Some(mut registration) = self.vec.iter_mut().find(|reg| reg.fd == fd) else {
                return Err(ErrorKind::NotFound.into());
            };

            let set = registration.events.contains(event);

            registration.events.remove(event);

            Ok(set)
        }

        fn set_fds(&self, fds: &mut Fds) -> io::Result<Option<RawFd>> {
            let mut max: Option<RawFd> = None;

            if let Some(event_fd) = self.event_fd {
                fds.set(event_fd, Event::Read);
                max = Some(max.map_or(event_fd, |max| max.max(event_fd)));
            }

            for registration in &self.vec {
                for event in EnumSet::ALL {
                    if registration.wakers[event as usize].is_some() {
                        fds.set(registration.fd, event);
                    }

                    max = Some(max.map_or(registration.fd, |max| max.max(registration.fd)));
                }
            }

            Ok(max)
        }

        fn update_events(&mut self, fds: &Fds) -> io::Result<()> {
            self.consume_notification()?;

            for registration in &mut self.vec {
                for event in EnumSet::ALL {
                    if fds.is_set(registration.fd, event) {
                        registration.events |= event;
                        if let Some(waker) = registration.wakers[event as usize].take() {
                            waker.wake();
                        }
                    }
                }
            }

            Ok(())
        }

        fn create_notification(&mut self) -> io::Result<bool> {
            if self.event_fd.is_none() {
                let handle = unsafe { crate::sys::eventfd(0, crate::sys::FNONBLOCK as _) };

                self.event_fd = Some(handle);

                Ok(true)
            } else {
                Ok(false)
            }
        }

        fn destroy_notification(&mut self) -> io::Result<bool> {
            if let Some(event_fd) = self.event_fd.take() {
                esp!(unsafe { crate::sys::close(event_fd) })
                    .map_err(|e| io::Error::from_raw_os_error(e.code()))?;

                Ok(true)
            } else {
                Ok(false)
            }
        }

        fn notify(&self) -> io::Result<bool> {
            if let Some(event_fd) = self.event_fd {
                esp!(unsafe {
                    crate::sys::write(
                        event_fd,
                        &u64::to_be_bytes(1_u64) as *const _ as *const _,
                        core::mem::size_of::<u64>(),
                    )
                })
                .map_err(|e| io::Error::from_raw_os_error(e.code()))?;

                Ok(true)
            } else {
                Ok(false)
            }
        }

        fn consume_notification(&mut self) -> io::Result<bool> {
            if let Some(event_fd) = self.event_fd {
                let mut buf = [0_u8; core::mem::size_of::<u64>()];

                loop {
                    let result = esp!(unsafe {
                        crate::sys::read(
                            event_fd,
                            &mut buf as *mut _ as *mut _,
                            core::mem::size_of::<u64>(),
                        )
                    });

                    if let Err(e) = &result {
                        if e.code() == crate::sys::EWOULDBLOCK as _ {
                            break;
                        }
                    }
                }

                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    /// Wake runner configuration
    #[derive(Clone, Debug)]
    pub struct VfsReactorConfig {
        pub task_name: &'static CStr,
        pub task_stack_size: usize,
        pub task_priority: u8,
        pub task_pin_to_core: Option<Core>,
    }

    impl VfsReactorConfig {
        pub const fn new() -> Self {
            Self {
                task_name: unsafe { CStr::from_bytes_with_nul_unchecked(b"VfsReactor\0") },
                task_stack_size: 1024,
                task_priority: 9,
                task_pin_to_core: None,
            }
        }
    }

    impl Default for VfsReactorConfig {
        fn default() -> Self {
            Self::new()
        }
    }

    pub struct VfsReactor<const N: usize> {
        registrations: std::sync::Mutex<Registrations<N>>,
        task_cs: CriticalSection,
        task: AtomicPtr<crate::sys::tskTaskControlBlock>,
        task_config: VfsReactorConfig,
    }

    impl<const N: usize> VfsReactor<N> {
        const fn new(config: VfsReactorConfig) -> Self {
            Self {
                registrations: std::sync::Mutex::new(Registrations::new()),
                task_cs: CriticalSection::new(),
                task: AtomicPtr::new(core::ptr::null_mut()),
                task_config: config,
            }
        }

        /// Returns `true` if the wake runner is started.
        pub fn is_started(&self) -> bool {
            !self.task.load(Ordering::SeqCst).is_null()
        }

        /// Starts the wake runner. Returns `false` if it had been already started.
        pub fn start(&'static self) -> io::Result<bool> {
            let _guard = self.task_cs.enter();

            if self.task.load(Ordering::SeqCst).is_null() {
                let task = unsafe {
                    crate::hal::task::create(
                        Self::task_run,
                        self.task_config.task_name,
                        self.task_config.task_stack_size,
                        self as *const _ as *const c_void as *mut _,
                        self.task_config.task_priority,
                        self.task_config.task_pin_to_core,
                    )
                    .map_err(|e| io::Error::from_raw_os_error(e.code()))?
                };

                self.task.store(task as _, Ordering::SeqCst);

                info!("IsrReactor {:?} started.", self.task_config.task_name);

                Ok(true)
            } else {
                Ok(false)
            }
        }

        /// Stops the wake runner. Returns `false` if it had been already stopped.
        pub fn stop(&self) -> bool {
            let _guard = self.task_cs.enter();

            let task = self.task.swap(core::ptr::null_mut(), Ordering::SeqCst);

            if !task.is_null() {
                unsafe {
                    crate::hal::task::destroy(task as _);
                }

                info!("IsrReactor {:?} stopped.", self.task_config.task_name);

                true
            } else {
                false
            }
        }

        extern "C" fn task_run(ctx: *mut c_void) {
            let this = unsafe { (ctx as *mut VfsReactor as *const VfsReactor).as_ref() }.unwrap();

            this.run();
        }

        pub(crate) fn register(&self, fd: RawFd) -> io::Result<()> {
            self.lock(|regs| regs.register(fd))
        }

        pub(crate) fn deregister(&self, fd: RawFd) -> io::Result<()> {
            self.lock(|regs| regs.deregister(fd))
        }

        pub(crate) fn set(&self, fd: RawFd, event: Event, waker: &Waker) -> io::Result<()> {
            self.lock(|regs| regs.set(fd, event, waker))
        }

        pub(crate) fn fetch(&self, fd: RawFd, event: Event) -> io::Result<bool> {
            self.lock(|regs| regs.fetch(fd, event))
        }

        fn run(&self) -> io::Result<()> {
            if !self.lock(Registrations::create_notification)? {
                Err(ErrorKind::AlreadyExists)?;
            }

            let result = loop {
                let result = self.select();

                if result.is_err() {
                    break result;
                }
            };

            if !self.lock(Registrations::destroy_notification)? {
                Err(ErrorKind::NotFound)?;
            }

            result
        }

        fn select(&self) -> io::Result<()> {
            let mut fds = Fds::new();

            if let Some(max) = self.lock(|inner| inner.set_fds(&mut fds))? {
                esp!(unsafe {
                    crate::sys::select(
                        max + 1,
                        &mut fds.read,
                        &mut fds.write,
                        &mut fds.except,
                        core::ptr::null_mut(),
                    )
                })
                .map_err(|e| io::Error::from_raw_os_error(e.code()))?;

                self.lock(|inner| inner.update_events(&fds))?;
            }

            Ok(())
        }

        fn lock<F, R>(&self, f: F) -> io::Result<R>
        where
            F: FnOnce(&mut Registrations) -> io::Result<R>,
        {
            let mut inner = self.registrations.lock().unwrap();

            f(&mut inner)
        }
    }

    pub static VFS_REACTOR: VfsReactor<MAX_REGISTRATIONS> =
        VfsReactor::new(VfsReactorConfig::new());
}
