use crate::sys::*;

use core::{marker::PhantomData, ops::Deref};

use esp_idf_hal::{
    gpio::{InputPin, OutputPin},
    peripheral::Peripheral,
    spi,
};

pub struct SpiDevice<'d> {
    configuration: sdspi_device_config_t,
    handle: sdspi_dev_handle_t,
    _p: PhantomData<&'d mut ()>,
}

impl Drop for SpiDevice<'_> {
    fn drop(&mut self) {
        let result = unsafe { sdspi_host_remove_device(self.handle) };

        if result != ESP_OK {
            panic!("Failed to remove SPI device");
        }
    }
}

impl<'d> SpiDevice<'d> {
    pub fn initialize_host() -> Result<(), EspError> {
        esp!(unsafe { sdspi_host_init() })
    }

    pub fn new<T>(
        _spi: impl Peripheral<P = T>,
        cs: impl Peripheral<P = impl OutputPin> + 'd,
        cd: Option<impl Peripheral<P = impl InputPin> + 'd>,
        wp: Option<impl Peripheral<P = impl InputPin> + 'd>,
        int: Option<impl Peripheral<P = impl InputPin> + 'd>,
        #[cfg(not(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
        )))] // For ESP-IDF v5.2 and later
        wp_polarity: Option<bool>,
    ) -> Result<Self, EspError>
    where
        T: spi::SpiAnyPins + spi::Spi,
    {
        let configuration = sdspi_device_config_t {
            host_id: T::device(),
            gpio_cs: cs.into_ref().deref().pin(),
            gpio_cd: cd.map(|cd| cd.into_ref().deref().pin()).unwrap_or(-1),
            gpio_wp: wp.map(|wp| wp.into_ref().deref().pin()).unwrap_or(-1),
            gpio_int: int.map(|int| int.into_ref().deref().pin()).unwrap_or(-1),
            #[cfg(not(any(
                esp_idf_version_major = "4",
                all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
            )))] // For ESP-IDF v5.2 and later
            gpio_wp_polarity: wp_polarity.unwrap_or(false), // Active when low
        };

        let mut handle: sdspi_dev_handle_t = 0;

        let result = unsafe {
            sdspi_host_init_device(&configuration as *const sdspi_device_config_t, &mut handle)
        };

        EspError::check_and_return(
            result,
            Self {
                configuration,
                handle,
                _p: PhantomData,
            },
        )
    }

    pub fn get_inner_handle(&self) -> &sdspi_dev_handle_t {
        &self.handle
    }

    pub fn set_clock(&mut self, clock: u32) -> Result<(), esp_err_t> {
        let result = unsafe { sdspi_host_set_card_clk(self.handle, clock) };

        if result == ESP_OK {
            Ok(())
        } else {
            Err(result)
        }
    }

    pub fn get_clock(&mut self) -> Result<u32, esp_err_t> {
        unimplemented!() // ! : Function not found in `esp-idf-sys`
    }

    pub fn enable_interrupt(&mut self) -> Result<(), esp_err_t> {
        let result = unsafe { sdspi_host_io_int_enable(self.handle) };

        if result == ESP_OK {
            Ok(())
        } else {
            Err(result)
        }
    }

    pub fn wait_interrupt(&mut self, timeout: u32) -> Result<(), esp_err_t> {
        let result = unsafe { sdspi_host_io_int_wait(self.handle, timeout) };

        if result == ESP_OK {
            Ok(())
        } else {
            Err(result)
        }
    }

    pub fn deinit_host() -> Result<(), esp_err_t> {
        let result = unsafe { sdspi_host_deinit() };

        if result == ESP_OK {
            Ok(())
        } else {
            Err(result)
        }
    }

    pub fn get_device_configuration(&self) -> &sdspi_device_config_t {
        &self.configuration
    }
}
