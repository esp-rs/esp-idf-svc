use core::convert::TryInto;

use esp_idf_sys::*;

pub fn micros_since_boot() -> u64 {
    unsafe { esp_timer_get_time() }.try_into().unwrap()
}
