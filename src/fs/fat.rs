use crate::sys::*;
use crate::sd::SdHost;

pub struct Fat(u8);

impl Fat {
    pub fn new(base_path: &str) -> Result<Self, esp_err_t> {
        Ok(Self(0))
    }
}