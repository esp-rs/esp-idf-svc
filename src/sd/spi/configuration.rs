use core::ops::Deref;

use crate::sys::*;
use esp_idf_hal::{gpio, peripheral, spi};

pub struct SpiDeviceConfiguration(sdspi_device_config_t);

impl From<sdspi_device_config_t> for SpiDeviceConfiguration {
    fn from(config: sdspi_device_config_t) -> Self {
        Self(config)
    }
}

impl From<SpiDeviceConfiguration> for sdspi_device_config_t {
    fn from(config: SpiDeviceConfiguration) -> Self {
        config.0
    }
}

impl Default for SpiDeviceConfiguration {
    fn default() -> Self {
        Self(sdspi_device_config_t {
            host_id: 0,
            gpio_cs: 0,
            gpio_cd: 0,
            gpio_wp: 0,
            gpio_int: 0,
        })
    }
}

impl SpiDeviceConfiguration {
    pub fn set_spi<T: spi::Spi>(&mut self, _: &mut T) -> &mut Self {
        self.0.host_id = T::device();
        self
    }

    pub fn set_cs_pin(
        &mut self,
        pin: impl peripheral::Peripheral<P = impl gpio::OutputPin>,
    ) -> &mut Self {
        let peripheral_reference = pin.into_ref();
        let pin = peripheral_reference.deref();
        self.0.gpio_cs = pin.pin();
        self
    }

    pub fn set_cd_pin(
        &mut self,
        pin: impl peripheral::Peripheral<P = impl gpio::InputPin>,
    ) -> &mut Self {
        let peripheral_reference = pin.into_ref();
        let pin = peripheral_reference.deref();
        self.0.gpio_cd = pin.pin();
        self
    }

    pub fn set_wp_pin(
        &mut self,
        pin: impl peripheral::Peripheral<P = impl gpio::InputPin>,
    ) -> &mut Self {
        let peripheral_reference = pin.into_ref();
        let pin = peripheral_reference.deref();
        self.0.gpio_wp = pin.pin();
        self
    }

    pub fn set_int_pin(
        &mut self,
        pin: impl peripheral::Peripheral<P = impl gpio::InputPin>,
    ) -> &mut Self {
        let peripheral_reference = pin.into_ref();
        let pin = peripheral_reference.deref();
        self.0.gpio_int = pin.pin();
        self
    }
}
