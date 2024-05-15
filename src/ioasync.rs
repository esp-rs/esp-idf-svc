//! Async I/O for the ESP IDF VFS layer.
//!
//! This crate provides [`Async`], an adapter for standard networking types (and [many other] types) to use in
//! async programs.
//!
//! # Implementation
//!
//! The first time [`Async`] is used, an ESP IDF task called "VfsReactor" will be spawned.
//! The purpose of this task is to wait for I/O events reported by the ESP IDF VFS kayer, and then
//! wake appropriate futures blocked on I/O when they can be resumed.
//!
//! To wait for the next I/O event, the task uses the [select] syscall available in the ESP IDF VFS layer.
//!
//! # Examples
//!
//! Connect to `example.com:80`, or time out after 10 seconds.
//!
//! ```
//! use esp_idf_svc::io::vfs::Async;
//!
//! use std::net::{TcpStream, ToSocketAddrs};
//!
//! # esp_idf_svc::hal::task::block_on(async {
//! let addr = "example.com:80".to_socket_addrs()?.next().unwrap();
//!
//! let stream = Async::<TcpStream>::connect(addr).await?;
//! # std::io::Result::Ok(()) });
//! ```

use core::future::{poll_fn, Future};
use core::pin::{pin, Pin};
use core::task::{Context, Poll, Waker};

use std::io::{self, IoSlice, IoSliceMut, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};

use super::reactor::{Event, VFS_REACTOR};

// use futures_io::{AsyncRead, AsyncWrite};
// use futures_lite::stream::{self, Stream};
// use futures_lite::{future, pin, ready};

// use rustix::io as rio;
// use rustix::net as rn;

// use crate::reactor::{Reactor, Registration, Source};

// mod driver;
// mod reactor;

// pub mod os;

// pub use driver::block_on;
// pub use reactor::{Readable, ReadableOwned, Writable, WritableOwned};

/// Async adapter for I/O types.
///
/// This type puts an I/O handle into non-blocking mode, registers it in
/// [epoll]/[kqueue]/[event ports]/[IOCP], and then provides an async interface for it.
///
/// [epoll]: https://en.wikipedia.org/wiki/Epoll
/// [kqueue]: https://en.wikipedia.org/wiki/Kqueue
/// [event ports]: https://illumos.org/man/port_create
/// [IOCP]: https://learn.microsoft.com/en-us/windows/win32/fileio/i-o-completion-ports
///
/// # Caveats
///
/// [`Async`] is a low-level primitive, and as such it comes with some caveats.
///
/// For higher-level primitives built on top of [`Async`], look into [`async-net`] or
/// [`async-process`] (on Unix).
///
/// The most notable caveat is that it is unsafe to access the inner I/O source mutably
/// using this primitive. Traits likes [`AsyncRead`] and [`AsyncWrite`] are not implemented by
/// default unless it is guaranteed that the resource won't be invalidated by reading or writing.
/// See the [`IoSafe`] trait for more information.
///
/// [`async-net`]: https://github.com/smol-rs/async-net
/// [`async-process`]: https://github.com/smol-rs/async-process
/// [`AsyncRead`]: https://docs.rs/futures-io/latest/futures_io/trait.AsyncRead.html
/// [`AsyncWrite`]: https://docs.rs/futures-io/latest/futures_io/trait.AsyncWrite.html
///
/// ### Supported types
///
/// [`Async`] supports all networking types, as well as some OS-specific file descriptors like
/// [timerfd] and [inotify].
///
/// However, do not use [`Async`] with types like [`File`][`std::fs::File`],
/// [`Stdin`][`std::io::Stdin`], [`Stdout`][`std::io::Stdout`], or [`Stderr`][`std::io::Stderr`]
/// because all operating systems have issues with them when put in non-blocking mode.
///
/// [timerfd]: https://github.com/smol-rs/async-io/blob/master/examples/linux-timerfd.rs
/// [inotify]: https://github.com/smol-rs/async-io/blob/master/examples/linux-inotify.rs
///
/// ### Concurrent I/O
///
/// Note that [`&Async<T>`][`Async`] implements [`AsyncRead`] and [`AsyncWrite`] if `&T`
/// implements those traits, which means tasks can concurrently read and write using shared
/// references.
///
/// But there is a catch: only one task can read a time, and only one task can write at a time. It
/// is okay to have two tasks where one is reading and the other is writing at the same time, but
/// it is not okay to have two tasks reading at the same time or writing at the same time. If you
/// try to do that, conflicting tasks will just keep waking each other in turn, thus wasting CPU
/// time.
///
/// Besides [`AsyncRead`] and [`AsyncWrite`], this caveat also applies to
/// [`poll_readable()`][`Async::poll_readable()`] and
/// [`poll_writable()`][`Async::poll_writable()`].
///
/// However, any number of tasks can be concurrently calling other methods like
/// [`readable()`][`Async::readable()`] or [`read_with()`][`Async::read_with()`].
///
/// ### Closing
///
/// Closing the write side of [`Async`] with [`close()`][`futures_lite::AsyncWriteExt::close()`]
/// simply flushes. If you want to shutdown a TCP or Unix socket, use
/// [`Shutdown`][`std::net::Shutdown`].
///
/// # Examples
///
/// Connect to a server and echo incoming messages back to the server:
///
/// ```no_run
/// use esp_idf_svc::io::vfs::Async;
/// use futures_lite::io;
/// use std::net::TcpStream;
///
/// # esp_idf_svc::hal::task::block_on(async {
/// // Connect to a local server.
/// let stream = Async::<TcpStream>::connect(([127, 0, 0, 1], 8000)).await?;
///
/// // Echo all messages from the read side of the stream into the write side.
/// io::copy(&stream, &stream).await?;
/// # std::io::Result::Ok(()) });
/// ```
///
/// You can use either predefined async methods or wrap blocking I/O operations in
/// [`Async::read_with()`], [`Async::read_with_mut()`], [`Async::write_with()`], and
/// [`Async::write_with_mut()`]:
///
/// ```no_run
/// use esp_idf_svc::io::vfs::Async;
/// use std::net::TcpListener;
///
/// # esp_idf_svc::hal::task::block_on(async {
/// let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
///
/// let (stream, addr) = listener.accept().await?;
/// # std::io::Result::Ok(()) });
/// ```
#[derive(Debug)]
pub struct Async<T> {
    io: T,
}

impl<T> Unpin for Async<T> {}

impl<T: AsFd> Async<T> {
    /// Creates an async I/O handle.
    ///
    /// This method will put the handle in non-blocking mode and register it in
    /// [epoll]/[kqueue]/[event ports]/[IOCP].
    ///
    /// On Unix systems, the handle must implement `AsFd`, while on Windows it must implement
    /// `AsSocket`.
    ///
    /// [epoll]: https://en.wikipedia.org/wiki/Epoll
    /// [kqueue]: https://en.wikipedia.org/wiki/Kqueue
    /// [event ports]: https://illumos.org/man/port_create
    /// [IOCP]: https://learn.microsoft.com/en-us/windows/win32/fileio/i-o-completion-ports
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::{SocketAddr, TcpListener};
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    /// let listener = Async::new(listener)?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn new(io: T) -> io::Result<Self> {
        // Put the file descriptor in non-blocking mode.
        set_nonblocking(io.as_fd())?;

        Self::new_nonblocking(io)
    }

    /// Creates an async I/O handle without setting it to non-blocking mode.
    ///
    /// This method will register the handle in [epoll]/[kqueue]/[event ports]/[IOCP].
    ///
    /// On Unix systems, the handle must implement `AsFd`, while on Windows it must implement
    /// `AsSocket`.
    ///
    /// [epoll]: https://en.wikipedia.org/wiki/Epoll
    /// [kqueue]: https://en.wikipedia.org/wiki/Kqueue
    /// [event ports]: https://illumos.org/man/port_create
    /// [IOCP]: https://learn.microsoft.com/en-us/windows/win32/fileio/i-o-completion-ports
    ///
    /// # Caveats
    ///
    /// The caller should ensure that the handle is set to non-blocking mode or that it is okay if
    /// it is not set. If not set to non-blocking mode, I/O operations may block the current thread
    /// and cause a deadlock in an asynchronous context.
    pub fn new_nonblocking(io: T) -> io::Result<Self> {
        VFS_REACTOR.start()?;
        VFS_REACTOR.register(io.as_fd())?;

        Ok(Self { io })
    }
}

impl<T: AsRawFd> AsRawFd for Async<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.get_ref().as_raw_fd()
    }
}

impl<T: AsFd> AsFd for Async<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.get_ref().as_fd()
    }
}

impl<T: AsFd + From<OwnedFd>> TryFrom<OwnedFd> for Async<T> {
    type Error = io::Error;

    fn try_from(value: OwnedFd) -> Result<Self, Self::Error> {
        Async::new(value.into())
    }
}

impl<T: Into<OwnedFd>> TryFrom<Async<T>> for OwnedFd {
    type Error = io::Error;

    fn try_from(value: Async<T>) -> Result<Self, Self::Error> {
        value.into_inner().map(Into::into)
    }
}

impl<T> Async<T> {
    /// Gets a reference to the inner I/O handle.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
    /// let inner = listener.get_ref();
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn get_ref(&self) -> &T {
        self.io.as_ref()
    }

    /// Gets a mutable reference to the inner I/O handle.
    ///
    /// # Safety
    ///
    /// The underlying I/O source must not be dropped using this function.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let mut listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
    /// let inner = unsafe { listener.get_mut() };
    /// # std::io::Result::Ok(()) });
    /// ```
    pub unsafe fn get_mut(&mut self) -> &mut T {
        self.io.as_mut().unwrap()
    }

    /// Unwraps the inner I/O handle.
    ///
    /// This method will **not** put the I/O handle back into blocking mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
    /// let inner = listener.into_inner()?;
    ///
    /// // Put the listener back into blocking mode.
    /// inner.set_nonblocking(false)?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn into_inner(mut self) -> io::Result<T> {
        VFS_REACTOR.deregister(self.io.as_fd())?;
        Ok(self.io)
    }

    /// Waits until the I/O handle is readable.
    ///
    /// This method completes when a read operation on this I/O handle wouldn't block.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let mut listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
    ///
    /// // Wait until a client can be accepted.
    /// listener.readable().await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn readable(&self) -> io::Result<()> {
        poll_fn(|cx| self.poll_readable(cx)).await
    }

    /// Waits until the I/O handle is writable.
    ///
    /// This method completes when a write operation on this I/O handle wouldn't block.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::{TcpStream, ToSocketAddrs};
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let addr = "example.com:80".to_socket_addrs()?.next().unwrap();
    /// let stream = Async::<TcpStream>::connect(addr).await?;
    ///
    /// // Wait until the stream is writable.
    /// stream.writable().await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn writable(&self) -> io::Result<()> {
        poll_fn(|cx| self.poll_writable(cx)).await
    }

    /// Polls the I/O handle for readability.
    ///
    /// When this method returns [`Poll::Ready`], that means the OS has delivered an event
    /// indicating readability since the last time this task has called the method and received
    /// [`Poll::Pending`].
    ///
    /// # Caveats
    ///
    /// Two different tasks should not call this method concurrently. Otherwise, conflicting tasks
    /// will just keep waking each other in turn, thus wasting CPU time.
    ///
    /// Note that the [`AsyncRead`] implementation for [`Async`] also uses this method.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use futures_lite::future;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let mut listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
    ///
    /// // Wait until a client can be accepted.
    /// future::poll_fn(|cx| listener.poll_readable(cx)).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if VFS_REACTOR.fetch(self.io.as_fd(), Event::Read) {
            Poll::Ready(Ok(()))
        } else {
            VFS_REACTOR.set(self.io.as_fd(), Event::Read, cx.waker())?;

            Poll::Pending
        }
    }

    /// Polls the I/O handle for writability.
    ///
    /// When this method returns [`Poll::Ready`], that means the OS has delivered an event
    /// indicating writability since the last time this task has called the method and received
    /// [`Poll::Pending`].
    ///
    /// # Caveats
    ///
    /// Two different tasks should not call this method concurrently. Otherwise, conflicting tasks
    /// will just keep waking each other in turn, thus wasting CPU time.
    ///
    /// Note that the [`AsyncWrite`] implementation for [`Async`] also uses this method.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use futures_lite::future;
    /// use std::net::{TcpStream, ToSocketAddrs};
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let addr = "example.com:80".to_socket_addrs()?.next().unwrap();
    /// let stream = Async::<TcpStream>::connect(addr).await?;
    ///
    /// // Wait until the stream is writable.
    /// future::poll_fn(|cx| stream.poll_writable(cx)).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if VFS_REACTOR.fetch(self.io.as_fd(), Event::Write) {
            Poll::Ready(Ok(()))
        } else {
            VFS_REACTOR.set(self.io.as_fd(), Event::Write, cx.waker())?;

            Poll::Pending
        }
    }

    /// Performs a read operation asynchronously.
    ///
    /// The I/O handle is registered in the reactor and put in non-blocking mode. This method
    /// invokes the `op` closure in a loop until it succeeds or returns an error other than
    /// [`io::ErrorKind::WouldBlock`]. In between iterations of the loop, it waits until the OS
    /// sends a notification that the I/O handle is readable.
    ///
    /// The closure receives a shared reference to the I/O handle.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
    ///
    /// // Accept a new client asynchronously.
    /// let (stream, addr) = listener.read_with(|l| l.accept()).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn read_with<R>(&self, op: impl FnMut(&T) -> io::Result<R>) -> io::Result<R> {
        VFS_REACTOR.fetch(self.io.as_fd(), Event::Read)?;

        let mut op = op;
        loop {
            match op(self.get_ref()) {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                res => return res,
            }
            optimistic(self.readable()).await?;
        }
    }

    /// Performs a read operation asynchronously.
    ///
    /// The I/O handle is registered in the reactor and put in non-blocking mode. This method
    /// invokes the `op` closure in a loop until it succeeds or returns an error other than
    /// [`io::ErrorKind::WouldBlock`]. In between iterations of the loop, it waits until the OS
    /// sends a notification that the I/O handle is readable.
    ///
    /// The closure receives a mutable reference to the I/O handle.
    ///
    /// # Safety
    ///
    /// In the closure, the underlying I/O source must not be dropped.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let mut listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
    ///
    /// // Accept a new client asynchronously.
    /// let (stream, addr) = unsafe { listener.read_with_mut(|l| l.accept()).await? };
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async unsafe fn read_with_mut<R>(
        &mut self,
        op: impl FnMut(&mut T) -> io::Result<R>,
    ) -> io::Result<R> {
        VFS_REACTOR.fetch(self.io.as_fd(), Event::Read)?;

        let mut op = op;
        loop {
            match op(self.get_mut()) {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                res => return res,
            }
            optimistic(self.readable()).await?;
        }
    }

    /// Performs a write operation asynchronously.
    ///
    /// The I/O handle is registered in the reactor and put in non-blocking mode. This method
    /// invokes the `op` closure in a loop until it succeeds or returns an error other than
    /// [`io::ErrorKind::WouldBlock`]. In between iterations of the loop, it waits until the OS
    /// sends a notification that the I/O handle is writable.
    ///
    /// The closure receives a shared reference to the I/O handle.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 8000))?;
    /// socket.get_ref().connect("127.0.0.1:9000")?;
    ///
    /// let msg = b"hello";
    /// let len = socket.write_with(|s| s.send(msg)).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn write_with<R>(&self, op: impl FnMut(&T) -> io::Result<R>) -> io::Result<R> {
        VFS_REACTOR.fetch(self.io.as_fd(), Event::Write)?;

        let mut op = op;
        loop {
            match op(self.get_ref()) {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                res => return res,
            }
            optimistic(self.writable()).await?;
        }
    }

    /// Performs a write operation asynchronously.
    ///
    /// The I/O handle is registered in the reactor and put in non-blocking mode. This method
    /// invokes the `op` closure in a loop until it succeeds or returns an error other than
    /// [`io::ErrorKind::WouldBlock`]. In between iterations of the loop, it waits until the OS
    /// sends a notification that the I/O handle is writable.
    ///
    /// # Safety
    ///
    /// The closure receives a mutable reference to the I/O handle. In the closure, the underlying
    /// I/O source must not be dropped.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let mut socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 8000))?;
    /// socket.get_ref().connect("127.0.0.1:9000")?;
    ///
    /// let msg = b"hello";
    /// let len = unsafe { socket.write_with_mut(|s| s.send(msg)).await? };
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async unsafe fn write_with_mut<R>(
        &mut self,
        op: impl FnMut(&mut T) -> io::Result<R>,
    ) -> io::Result<R> {
        VFS_REACTOR.fetch(self.io.as_fd(), Event::Write)?;

        let mut op = op;
        loop {
            match op(self.get_mut()) {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                res => return res,
            }
            optimistic(self.writable()).await?;
        }
    }
}

impl<T> AsRef<T> for Async<T> {
    fn as_ref(&self) -> &T {
        self.get_ref()
    }
}

impl<T> Drop for Async<T> {
    fn drop(&mut self) {
        VFS_REACTOR.deregister(self.io.as_fd()).ok();
    }
}

/// Types whose I/O trait implementations do not drop the underlying I/O source.
///
/// The resource contained inside of the [`Async`] cannot be invalidated. This invalidation can
/// happen if the inner resource (the [`TcpStream`], [`UnixListener`] or other `T`) is moved out
/// and dropped before the [`Async`]. Because of this, functions that grant mutable access to
/// the inner type are unsafe, as there is no way to guarantee that the source won't be dropped
/// and a dangling handle won't be left behind.
///
/// Unfortunately this extends to implementations of [`Read`] and [`Write`]. Since methods on those
/// traits take `&mut`, there is no guarantee that the implementor of those traits won't move the
/// source out while the method is being run.
///
/// This trait is an antidote to this predicament. By implementing this trait, the user pledges
/// that using any I/O traits won't destroy the source. This way, [`Async`] can implement the
/// `async` version of these I/O traits, like [`AsyncRead`] and [`AsyncWrite`].
///
/// # Safety
///
/// Any I/O trait implementations for this type must not drop the underlying I/O source. Traits
/// affected by this trait include [`Read`], [`Write`], [`Seek`] and [`BufRead`].
///
/// This trait is implemented by default on top of `libstd` types. In addition, it is implemented
/// for immutable reference types, as it is impossible to invalidate any outstanding references
/// while holding an immutable reference, even with interior mutability. As Rust's current pinning
/// system relies on similar guarantees, I believe that this approach is robust.
///
/// [`BufRead`]: https://doc.rust-lang.org/std/io/trait.BufRead.html
/// [`Read`]: https://doc.rust-lang.org/std/io/trait.Read.html
/// [`Seek`]: https://doc.rust-lang.org/std/io/trait.Seek.html
/// [`Write`]: https://doc.rust-lang.org/std/io/trait.Write.html
///
/// [`AsyncRead`]: https://docs.rs/futures-io/latest/futures_io/trait.AsyncRead.html
/// [`AsyncWrite`]: https://docs.rs/futures-io/latest/futures_io/trait.AsyncWrite.html
pub unsafe trait IoSafe {}

/// Reference types can't be mutated.
///
/// The worst thing that can happen is that external state is used to change what kind of pointer
/// `as_fd()` returns. For instance:
///
/// ```
/// # #[cfg(unix)] {
/// use std::cell::Cell;
/// use std::net::TcpStream;
/// use std::os::unix::io::{AsFd, BorrowedFd};
///
/// struct Bar {
///     flag: Cell<bool>,
///     a: TcpStream,
///     b: TcpStream
/// }
///
/// impl AsFd for Bar {
///     fn as_fd(&self) -> BorrowedFd<'_> {
///         if self.flag.replace(!self.flag.get()) {
///             self.a.as_fd()
///         } else {
///             self.b.as_fd()
///         }
///     }
/// }
/// # }
/// ```
///
/// We solve this problem by only calling `as_fd()` once to get the original source. Implementations
/// like this are considered buggy (but not unsound) and are thus not really supported by `async-io`.
unsafe impl<T: ?Sized> IoSafe for &T {}

// Can be implemented on top of libstd types.
unsafe impl IoSafe for std::fs::File {}
unsafe impl IoSafe for std::io::Stderr {}
unsafe impl IoSafe for std::io::Stdin {}
unsafe impl IoSafe for std::io::Stdout {}
unsafe impl IoSafe for std::io::StderrLock<'_> {}
unsafe impl IoSafe for std::io::StdinLock<'_> {}
unsafe impl IoSafe for std::io::StdoutLock<'_> {}
unsafe impl IoSafe for std::net::TcpStream {}
unsafe impl IoSafe for std::process::ChildStdin {}
unsafe impl IoSafe for std::process::ChildStdout {}
unsafe impl IoSafe for std::process::ChildStderr {}

unsafe impl<T: IoSafe + Read> IoSafe for std::io::BufReader<T> {}
unsafe impl<T: IoSafe + Write> IoSafe for std::io::BufWriter<T> {}
unsafe impl<T: IoSafe + Write> IoSafe for std::io::LineWriter<T> {}
unsafe impl<T: IoSafe + ?Sized> IoSafe for &mut T {}
unsafe impl<T: IoSafe + ?Sized> IoSafe for alloc::boxed::Box<T> {}
unsafe impl<T: Clone + IoSafe + ?Sized> IoSafe for std::borrow::Cow<'_, T> {}

// impl<T: IoSafe + Read> AsyncRead for Async<T> {
//     fn poll_read(
//         mut self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         buf: &mut [u8],
//     ) -> Poll<io::Result<usize>> {
//         loop {
//             match unsafe { (*self).get_mut() }.read(buf) {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_readable(cx))?;
//         }
//     }

//     fn poll_read_vectored(
//         mut self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         bufs: &mut [IoSliceMut<'_>],
//     ) -> Poll<io::Result<usize>> {
//         loop {
//             match unsafe { (*self).get_mut() }.read_vectored(bufs) {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_readable(cx))?;
//         }
//     }
// }

// // Since this is through a reference, we can't mutate the inner I/O source.
// // Therefore this is safe!
// impl<T> AsyncRead for &Async<T>
// where
//     for<'a> &'a T: Read,
// {
//     fn poll_read(
//         self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         buf: &mut [u8],
//     ) -> Poll<io::Result<usize>> {
//         loop {
//             match (*self).get_ref().read(buf) {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_readable(cx))?;
//         }
//     }

//     fn poll_read_vectored(
//         self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         bufs: &mut [IoSliceMut<'_>],
//     ) -> Poll<io::Result<usize>> {
//         loop {
//             match (*self).get_ref().read_vectored(bufs) {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_readable(cx))?;
//         }
//     }
// }

// impl<T: IoSafe + Write> AsyncWrite for Async<T> {
//     fn poll_write(
//         mut self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         buf: &[u8],
//     ) -> Poll<io::Result<usize>> {
//         loop {
//             match unsafe { (*self).get_mut() }.write(buf) {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_writable(cx))?;
//         }
//     }

//     fn poll_write_vectored(
//         mut self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         bufs: &[IoSlice<'_>],
//     ) -> Poll<io::Result<usize>> {
//         loop {
//             match unsafe { (*self).get_mut() }.write_vectored(bufs) {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_writable(cx))?;
//         }
//     }

//     fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
//         loop {
//             match unsafe { (*self).get_mut() }.flush() {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_writable(cx))?;
//         }
//     }

//     fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
//         self.poll_flush(cx)
//     }
// }

// impl<T> AsyncWrite for &Async<T>
// where
//     for<'a> &'a T: Write,
// {
//     fn poll_write(
//         self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         buf: &[u8],
//     ) -> Poll<io::Result<usize>> {
//         loop {
//             match (*self).get_ref().write(buf) {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_writable(cx))?;
//         }
//     }

//     fn poll_write_vectored(
//         self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         bufs: &[IoSlice<'_>],
//     ) -> Poll<io::Result<usize>> {
//         loop {
//             match (*self).get_ref().write_vectored(bufs) {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_writable(cx))?;
//         }
//     }

//     fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
//         loop {
//             match (*self).get_ref().flush() {
//                 Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
//                 res => return Poll::Ready(res),
//             }
//             ready!(self.poll_writable(cx))?;
//         }
//     }

//     fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
//         self.poll_flush(cx)
//     }
// }

impl Async<TcpListener> {
    /// Creates a TCP listener bound to the specified address.
    ///
    /// Binding with port number 0 will request an available port from the OS.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 0))?;
    /// println!("Listening on {}", listener.get_ref().local_addr()?);
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn bind<A: Into<SocketAddr>>(addr: A) -> io::Result<Async<TcpListener>> {
        let addr = addr.into();
        Async::new(TcpListener::bind(addr)?)
    }

    /// Accepts a new incoming TCP connection.
    ///
    /// When a connection is established, it will be returned as a TCP stream together with its
    /// remote address.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::TcpListener;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 8000))?;
    /// let (stream, addr) = listener.accept().await?;
    /// println!("Accepted client: {}", addr);
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn accept(&self) -> io::Result<(Async<TcpStream>, SocketAddr)> {
        let (stream, addr) = self.read_with(|io| io.accept()).await?;
        Ok((Async::new(stream)?, addr))
    }
}

impl TryFrom<std::net::TcpListener> for Async<std::net::TcpListener> {
    type Error = io::Error;

    fn try_from(listener: std::net::TcpListener) -> io::Result<Self> {
        Async::new(listener)
    }
}

impl Async<TcpStream> {
    /// Creates a TCP connection to the specified address.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::{TcpStream, ToSocketAddrs};
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let addr = "example.com:80".to_socket_addrs()?.next().unwrap();
    /// let stream = Async::<TcpStream>::connect(addr).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn connect<A: Into<SocketAddr>>(addr: A) -> io::Result<Async<TcpStream>> {
        // Figure out how to handle this address.
        let addr = addr.into();
        let (domain, sock_addr) = match addr {
            SocketAddr::V4(v4) => (rn::AddressFamily::INET, rn::SocketAddrAny::V4(v4)),
            SocketAddr::V6(v6) => (rn::AddressFamily::INET6, rn::SocketAddrAny::V6(v6)),
        };

        // Begin async connect.
        let socket = connect(sock_addr, domain, Some(rn::ipproto::TCP))?;
        // Use new_nonblocking because connect already sets socket to non-blocking mode.
        let stream = Async::new_nonblocking(TcpStream::from(socket))?;

        // The stream becomes writable when connected.
        stream.writable().await?;

        // Check if there was an error while connecting.
        match stream.get_ref().take_error()? {
            None => Ok(stream),
            Some(err) => Err(err),
        }
    }

    /// Reads data from the stream without removing it from the buffer.
    ///
    /// Returns the number of bytes read. Successive calls of this method read the same data.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use futures_lite::{io::AsyncWriteExt, stream::StreamExt};
    /// use std::net::{TcpStream, ToSocketAddrs};
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let addr = "example.com:80".to_socket_addrs()?.next().unwrap();
    /// let mut stream = Async::<TcpStream>::connect(addr).await?;
    ///
    /// stream
    ///     .write_all(b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n")
    ///     .await?;
    ///
    /// let mut buf = [0u8; 1024];
    /// let len = stream.peek(&mut buf).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn peek(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_with(|io| io.peek(buf)).await
    }
}

impl TryFrom<std::net::TcpStream> for Async<std::net::TcpStream> {
    type Error = io::Error;

    fn try_from(stream: std::net::TcpStream) -> io::Result<Self> {
        Async::new(stream)
    }
}

impl Async<UdpSocket> {
    /// Creates a UDP socket bound to the specified address.
    ///
    /// Binding with port number 0 will request an available port from the OS.
    ///
    /// # Examples
    ///
    /// ```
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 0))?;
    /// println!("Bound to {}", socket.get_ref().local_addr()?);
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn bind<A: Into<SocketAddr>>(addr: A) -> io::Result<Async<UdpSocket>> {
        let addr = addr.into();
        Async::new(UdpSocket::bind(addr)?)
    }

    /// Receives a single datagram message.
    ///
    /// Returns the number of bytes read and the address the message came from.
    ///
    /// This method must be called with a valid byte slice of sufficient size to hold the message.
    /// If the message is too long to fit, excess bytes may get discarded.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 8000))?;
    ///
    /// let mut buf = [0u8; 1024];
    /// let (len, addr) = socket.recv_from(&mut buf).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.read_with(|io| io.recv_from(buf)).await
    }

    /// Receives a single datagram message without removing it from the queue.
    ///
    /// Returns the number of bytes read and the address the message came from.
    ///
    /// This method must be called with a valid byte slice of sufficient size to hold the message.
    /// If the message is too long to fit, excess bytes may get discarded.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 8000))?;
    ///
    /// let mut buf = [0u8; 1024];
    /// let (len, addr) = socket.peek_from(&mut buf).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn peek_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.read_with(|io| io.peek_from(buf)).await
    }

    /// Sends data to the specified address.
    ///
    /// Returns the number of bytes writen.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 0))?;
    /// let addr = socket.get_ref().local_addr()?;
    ///
    /// let msg = b"hello";
    /// let len = socket.send_to(msg, addr).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn send_to<A: Into<SocketAddr>>(&self, buf: &[u8], addr: A) -> io::Result<usize> {
        let addr = addr.into();
        self.write_with(|io| io.send_to(buf, addr)).await
    }

    /// Receives a single datagram message from the connected peer.
    ///
    /// Returns the number of bytes read.
    ///
    /// This method must be called with a valid byte slice of sufficient size to hold the message.
    /// If the message is too long to fit, excess bytes may get discarded.
    ///
    /// The [`connect`][`UdpSocket::connect()`] method connects this socket to a remote address.
    /// This method will fail if the socket is not connected.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 8000))?;
    /// socket.get_ref().connect("127.0.0.1:9000")?;
    ///
    /// let mut buf = [0u8; 1024];
    /// let len = socket.recv(&mut buf).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_with(|io| io.recv(buf)).await
    }

    /// Receives a single datagram message from the connected peer without removing it from the
    /// queue.
    ///
    /// Returns the number of bytes read and the address the message came from.
    ///
    /// This method must be called with a valid byte slice of sufficient size to hold the message.
    /// If the message is too long to fit, excess bytes may get discarded.
    ///
    /// The [`connect`][`UdpSocket::connect()`] method connects this socket to a remote address.
    /// This method will fail if the socket is not connected.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 8000))?;
    /// socket.get_ref().connect("127.0.0.1:9000")?;
    ///
    /// let mut buf = [0u8; 1024];
    /// let len = socket.peek(&mut buf).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn peek(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_with(|io| io.peek(buf)).await
    }

    /// Sends data to the connected peer.
    ///
    /// Returns the number of bytes written.
    ///
    /// The [`connect`][`UdpSocket::connect()`] method connects this socket to a remote address.
    /// This method will fail if the socket is not connected.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use esp_idf_svc::io::vfs::Async;
    /// use std::net::UdpSocket;
    ///
    /// # esp_idf_svc::hal::task::block_on(async {
    /// let socket = Async::<UdpSocket>::bind(([127, 0, 0, 1], 8000))?;
    /// socket.get_ref().connect("127.0.0.1:9000")?;
    ///
    /// let msg = b"hello";
    /// let len = socket.send(msg).await?;
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.write_with(|io| io.send(buf)).await
    }
}

impl TryFrom<std::net::UdpSocket> for Async<std::net::UdpSocket> {
    type Error = io::Error;

    fn try_from(socket: std::net::UdpSocket) -> io::Result<Self> {
        Async::new(socket)
    }
}

/// Polls a future once, waits for a wakeup, and then optimistically assumes the future is ready.
async fn optimistic(fut: impl Future<Output = io::Result<()>>) -> io::Result<()> {
    let mut polled = false;
    pin!(fut);

    poll_fn(|cx| {
        if !polled {
            polled = true;
            fut.as_mut().poll(cx)
        } else {
            Poll::Ready(Ok(()))
        }
    })
    .await
}

fn connect(
    addr: rn::SocketAddrAny,
    domain: rn::AddressFamily,
    protocol: Option<rn::Protocol>,
) -> io::Result<rustix::fd::OwnedFd> {
    setup_networking();
    let socket = {
        let flags = rn::SocketFlags::empty();

        // Create the socket.
        let socket = rn::socket_with(domain, rn::SocketType::STREAM, flags, protocol)?;

        // Set non-blocking mode.
        set_nonblocking(socket.as_fd())?;

        socket
    };

    match rn::connect_any(&socket, &addr) {
        Ok(_) => {}
        Err(rio::Errno::INPROGRESS) => {}
        Err(rio::Errno::AGAIN) | Err(rio::Errno::WOULDBLOCK) => {}
        Err(err) => return Err(err.into()),
    }

    Ok(socket)
}

fn setup_networking() {}

fn set_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    let previous = unsafe { crate::sys::fcntl(crate::sys::F_GETFL) };
    let new = previous | crate::sys::FNONBLOCK;
    if new != previous {
        crate::sys::esp!(unsafe {
            crate::sys::fcntl(fd, crate::sys::F_SETFL, new);
        })?;
    }

    Ok(())
}
