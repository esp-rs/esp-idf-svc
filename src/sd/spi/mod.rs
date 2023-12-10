use crate::sys::*;
use core::ops::Deref;
use std::sync::Mutex;

pub mod configuration;
pub use configuration::*;

pub struct SpiDevice(Mutex<sdspi_dev_handle_t>);

impl From<sdspi_dev_handle_t> for SpiDevice {
    fn from(handle: sdspi_dev_handle_t) -> Self {
        Self(Mutex::new(handle))
    }
}

impl From<&mut SpiDevice> for sdspi_dev_handle_t {
    fn from(device: &mut SpiDevice) -> Self {
        let device = match device.0.lock() {
            Ok(device) => device,
            Err(_) => panic!("Failed to lock SPI device"),
        };
        *device.deref()
    }
}

impl Drop for SpiDevice {
    fn drop(&mut self) {
        let device = match self.0.lock() {
            Ok(device) => device,
            Err(_) => panic!("Failed to lock SPI device"),
        };
        unsafe {
            sdspi_host_remove_device(*device.deref());
        }
    }
}

impl SpiDevice {
    pub fn initialize(configuration: &mut SpiDeviceConfiguration) -> Result<Self, esp_err_t> {
        let mut handle: sdspi_dev_handle_t = 0;

        let result = unsafe {
            sdspi_host_init_device(
                configuration.get_configuration() as *const sdspi_device_config_t,
                &mut handle,
            )
        };

        if result == ESP_OK {
            Ok(handle.into())
        } else {
            Err(result)
        }
    }

    pub fn set_clock(&mut self, clock: u32) -> Result<(), esp_err_t> {
        let device = match self.0.lock() {
            Ok(device) => device,
            Err(_) => panic!("Failed to lock SPI device"),
        };
        let result = unsafe { sdspi_host_set_card_clk(*device.deref(), clock) };
        if result == ESP_OK {
            Ok(())
        } else {
            Err(result)
        }
    }

    pub fn get_clock(&mut self) -> Result<u32, esp_err_t> {
        unimplemented!() // ! : Function not found in `esp-idf-sys`
    }


}
