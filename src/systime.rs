//! System time
use core::time::Duration;

use crate::sys::*;

/// Client for the ESP system time
///
/// If you have enabled the `std` feature, you can also call
/// the standard `std::time::SystemTime` API in Rust.
pub struct EspSystemTime;

impl EspSystemTime {
    /// Return the current system time
    pub fn now(&self) -> Duration {
        let mut tv_now = timeval::default();

        unsafe {
            gettimeofday(core::ptr::addr_of_mut!(tv_now), core::ptr::null_mut());
        }

        Duration::from_micros(tv_now.tv_sec as u64 * 1_000_000_u64 + tv_now.tv_usec as u64)
    }
}
