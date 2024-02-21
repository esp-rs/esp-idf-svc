/// Zero-copy blocking SPSC primitive for sharing a mutable reference owned by one thread into another.
/// Useful as a rendezvous point between two threads: one - sharing, and the other - using the shared mutable reference.
///
/// The using thread can wait for the shared reference in an asynchronous way as well.
///
/// Note that - strictly speaking - the priitive is MPSC in the sense that multiple threads can share (i.e. produce) mutable references.
use super::mutex::{Condvar, Mutex};

extern crate alloc;
use alloc::sync::{Arc, Weak};

use esp_idf_hal::task::asynch::Notification;

use log::{debug, info};

pub struct Receiver<T>
where
    T: Send + 'static,
{
    blocking_notify: Condvar,
    state: Mutex<State<T>>,
    notify_full: Notification,
}

impl<T> Receiver<T>
where
    T: Send + 'static,
{
    pub fn get_shared(&self) -> Option<&mut T> {
        let mut guard = self.state.lock();
        loop {
            match &mut *guard {
                State::Empty => guard = self.blocking_notify.wait(guard),
                State::Quit => break None,
                State::Data(data) => break unsafe { data.as_mut() },
            }
        }
    }

    pub async fn get_shared_async(&self) -> Option<&mut T> {
        loop {
            {
                let mut guard = self.state.lock();

                match &mut *guard {
                    State::Empty => (),
                    State::Quit => return None,
                    State::Data(data) => return unsafe { data.as_mut() },
                }
            }

            self.notify_full.wait().await;
        }
    }

    pub fn done(&self) {
        let mut guard = self.state.lock();

        if matches!(&*guard, State::Data(_)) {
            *guard = State::Empty;
            self.blocking_notify.notify_all();
        }
    }
}

impl<T> Drop for Receiver<T>
where
    T: Send + 'static,
{
    fn drop(&mut self) {
        let mut guard = self.state.lock();
        *guard = State::Quit;
        self.blocking_notify.notify_all();
    }
}

unsafe impl<T> Send for Receiver<T> where T: Send + 'static {}
unsafe impl<T> Sync for Receiver<T> where T: Send + 'static {}

pub struct Channel<T>(Weak<Receiver<T>>)
where
    T: Send + 'static;

impl<T> Channel<T>
where
    T: Send + 'static,
{
    pub fn new() -> (Self, Arc<Receiver<T>>) {
        let receiver = Arc::new(Receiver {
            blocking_notify: Condvar::new(),
            state: Mutex::new(State::Empty),
            notify_full: Notification::new(),
        });

        let this = Self(Arc::downgrade(&receiver));

        (this, receiver)
    }

    pub fn share(&self, mut data: &mut T) -> bool {
        self.set(State::Data(data))
    }

    fn set(&self, data: State<T>) -> bool {
        if let Some(receiver) = self.0.upgrade() {
            let mut guard = receiver.state.lock();
            loop {
                match &*guard {
                    State::Empty => {
                        self.set_data(&mut guard, data);
                        receiver.blocking_notify.notify_all();
                        receiver.notify_full.notify_lsb();
                        break;
                    }
                    State::Quit => return false,
                    State::Data(_) => guard = receiver.blocking_notify.wait(guard),
                }
            }

            loop {
                match &*guard {
                    State::Empty | State::Quit => break,
                    State::Data(_) => guard = receiver.blocking_notify.wait(guard),
                }
            }

            true
        } else {
            false // same as State::Quit
        }
    }

    fn set_data(&self, cell: &mut State<T>, data: State<T>) {
        *cell = data;
    }
}

impl<T> Drop for Channel<T>
where
    T: Send + 'static,
{
    fn drop(&mut self) {
        self.set(State::Quit);
    }
}

unsafe impl<T> Send for Channel<T> where T: Send + 'static {}

#[derive(Copy, Clone, Debug)]
enum State<T> {
    Empty,
    Data(*mut T),
    Quit,
}
