pub use embedded_svc::utils::io as utils;
pub use esp_idf_hal::io::*;

#[cfg(esp_idf_comp_vfs_enabled)]
pub mod vfs {
    use crate::sys;

    /// Represents a mounted EventFD pseudo-filesystem.
    ///
    /// Operating on this filesystem is done only via the native, unsafe `sys::eventfd_*` function.
    pub struct MountedEventFs(());

    impl MountedEventFs {
        /// Mount the EventFD pseudo-filesystem.
        ///
        /// # Arguments
        /// - `max_fds`: The maximum number of file descriptors to allocate.
        #[allow(clippy::needless_update)]
        pub fn mount(max_fds: usize) -> Result<Self, sys::EspError> {
            sys::esp!(unsafe {
                sys::esp_vfs_eventfd_register(&sys::esp_vfs_eventfd_config_t {
                    max_fds: max_fds as _,
                    ..Default::default()
                })
            })?;

            Ok(Self(()))
        }
    }

    impl Drop for MountedEventFs {
        fn drop(&mut self) {
            sys::esp!(unsafe { sys::esp_vfs_eventfd_unregister() }).unwrap();
        }
    }

    /// Represents a mounted FAT filesystem.
    #[cfg(feature = "experimental")]
    pub struct MountedFatFs<T> {
        _handle: *mut sys::FATFS,
        _fatfs: T,
        path: alloc::ffi::CString,
        drive: u8,
    }

    #[cfg(feature = "experimental")]
    impl<T> MountedFatFs<T> {
        /// Mount a FAT filesystem.
        ///
        /// # Arguments
        /// - `fatfs`: The FAT filesystem instance to mount.
        /// - `path`: The path to mount the filesystem at.
        /// - `max_fds`: The maximum number of file descriptors to allocate.
        pub fn mount<H>(mut fatfs: T, path: &str, max_fds: usize) -> Result<Self, sys::EspError>
        where
            T: core::borrow::BorrowMut<crate::fs::fat::FatFs<H>>,
        {
            let path = crate::private::cstr::to_cstring_arg(path)?;
            let drive_path = fatfs.borrow_mut().drive_path();

            let mut handle = core::ptr::null_mut();

            sys::esp!(unsafe {
                sys::esp_vfs_fat_register(
                    path.as_ptr(),
                    drive_path.as_ptr(),
                    max_fds as _,
                    &mut handle,
                )
            })?;

            unsafe {
                sys::f_mount(handle, drive_path.as_ptr(), 0); // TODO
            }

            let drive = fatfs.borrow_mut().drive();

            Ok(Self {
                _handle: handle,
                _fatfs: fatfs,
                path,
                drive,
            })
        }
    }

    #[cfg(feature = "experimental")]
    impl<T> Drop for MountedFatFs<T> {
        fn drop(&mut self) {
            let drive_path = crate::fs::fat::FatFs::<()>::drive_path_from(self.drive);

            unsafe {
                sys::f_mount(core::ptr::null_mut(), drive_path.as_ptr(), 0);
            }

            sys::esp!(unsafe { sys::esp_vfs_fat_unregister_path(self.path.as_ptr()) }).unwrap();
        }
    }
}
