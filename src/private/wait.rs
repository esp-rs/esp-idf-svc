use core::time::Duration;
#[cfg(feature = "std")]
use std::sync::{Condvar, Mutex};

use embedded_svc::mutex::Mutex as _;
#[cfg(not(feature = "std"))]
use esp_idf_sys::*;

#[cfg(not(feature = "std"))]
use super::time::micros_since_boot;

pub struct Waiter {
    #[cfg(feature = "std")]
    cvar: Condvar,
    #[cfg(feature = "std")]
    running: Mutex<bool>,
    #[cfg(not(feature = "std"))]
    running: EspMutex<bool>,
}

impl Waiter {
    pub fn new() -> Self {
        Waiter {
            #[cfg(feature = "std")]
            cvar: Condvar::new(),
            #[cfg(feature = "std")]
            running: Mutex::new(false),
            #[cfg(not(feature = "std"))]
            running: EspMutex::new(false),
        }
    }

    pub fn start(&self) {
        self.running.with_lock(|running| *running = true);
    }

    #[cfg(feature = "std")]
    pub fn wait(&self) {
        if !self.running.with_lock(|running| *running) {
            return;
        }

        let _running = self
            .cvar
            .wait_while(self.running.lock().unwrap(), |running| *running)
            .unwrap();
    }

    #[cfg(not(feature = "std"))]
    pub fn wait(&self) {
        while self.running.with_lock(|running| *running) {
            unsafe { vTaskDelay(500) };
        }
    }

    /// return = !timeout (= success)
    #[cfg(feature = "std")]
    pub fn wait_timeout(&self, dur: Duration) -> bool {
        if !self.running.with_lock(|running| *running) {
            return true;
        }

        let (_running, res) = self
            .cvar
            .wait_timeout_while(self.running.lock().unwrap(), dur, |running| *running)
            .unwrap();

        return !res.timed_out();
    }

    /// return = !timeout (= success)
    #[cfg(not(feature = "std"))]
    pub fn wait_timeout(&self, dur: Duration) {
        let now = micros_since_boot();
        let end = now + dur.as_micros();

        while self.running.with_lock(|running| *running) {
            if micros_since_boot() > end {
                return false;
            }
            unsafe { vTaskDelay(500) };
        }

        return true;
    }

    #[cfg(feature = "std")]
    pub fn notify(&self) {
        *self.running.lock().unwrap() = false;
        self.cvar.notify_all();
    }

    #[cfg(not(feature = "std"))]
    pub fn notify(&self) {
        self.running.with_lock(|running| *running = false);
    }
}
