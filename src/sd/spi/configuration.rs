use crate::sys::*;
use esp_idf_hal::{
    gpio::{self, Pin},
    spi,
};

pub struct SpiDeviceConfiguration(sdspi_device_config_t);

impl From<sdspi_device_config_t> for SpiDeviceConfiguration {
    fn from(config: sdspi_device_config_t) -> Self {
        Self(config)
    }
}

impl Into<sdspi_device_config_t> for SpiDeviceConfiguration {
    fn into(self) -> sdspi_device_config_t {
        self.0
    }
}

impl SpiDeviceConfiguration {
    pub fn new() -> Self {
        Self(sdspi_device_config_t {
            host_id: 0,
            gpio_cs: 0,
            gpio_cd: 0,
            gpio_wp: 0,
            gpio_int: 0,
        })
    }

    pub fn set_spi<T: spi::Spi>(&mut self, _: &mut T) -> &mut Self {
        self.0.host_id = T::device();
        self
    }

    pub fn set_cs_pin<T: gpio::OutputPin>(&mut self, pin: T) -> &mut Self {
        let cs_pin = pin.downgrade_output();
        self.0.gpio_cs = cs_pin.pin();
        self
    }

    pub fn set_cd_pin<T: gpio::InputPin>(&mut self, pin: &mut T) -> &mut Self {
        self.0.gpio_cd = pin.downgrade_input().pin();
        self
    }

    pub fn set_wp_pin<T: gpio::InputPin>(&mut self, pin: &mut T) -> &mut Self {
        self.0.gpio_wp = pin.downgrade_input().pin();
        self
    }

    pub fn set_int_pin<T: gpio::InputPin>(&mut self, pin: &mut T) -> &mut Self {
        self.0.gpio_int = pin.downgrade_input().pin();
        self
    }
}
