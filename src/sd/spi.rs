use crate::sys::*;

use core::ops::Deref;

use esp_idf_hal::{gpio, peripheral, spi};

pub struct SpiDeviceBuilder(sdspi_device_config_t);

impl From<sdspi_device_config_t> for SpiDeviceBuilder {
    fn from(config: sdspi_device_config_t) -> Self {
        Self(config)
    }
}

impl Default for SpiDeviceBuilder {
    fn default() -> Self {
        Self(sdspi_device_config_t {
            host_id: spi_host_device_t_SPI2_HOST,
            gpio_cs: gpio_num_t_GPIO_NUM_13,
            gpio_cd: gpio_num_t_GPIO_NUM_NC,
            gpio_wp: gpio_num_t_GPIO_NUM_NC,
            gpio_int: gpio_num_t_GPIO_NUM_NC,
            gpio_wp_polarity: false, // Active when low
        })
    }
}

impl SpiDeviceBuilder {
    pub fn set_spi<T: spi::SpiAnyPins + spi::Spi>(
        &mut self,
        _spi: impl peripheral::Peripheral<P = T>,
    ) -> &mut Self {
        self.0.host_id = T::device();
        self
    }

    pub fn set_cs_pin(
        mut self,
        pin: impl peripheral::Peripheral<P = impl gpio::OutputPin>,
    ) -> Self {
        let peripheral_reference = pin.into_ref();
        let pin = peripheral_reference.deref();
        self.0.gpio_cs = pin.pin();
        self
    }

    pub fn set_cd_pin(mut self, pin: impl peripheral::Peripheral<P = impl gpio::InputPin>) -> Self {
        let peripheral_reference = pin.into_ref();
        let pin = peripheral_reference.deref();
        self.0.gpio_cd = pin.pin();
        self
    }

    pub fn set_wp_pin(mut self, pin: impl peripheral::Peripheral<P = impl gpio::InputPin>) -> Self {
        let peripheral_reference = pin.into_ref();
        let pin = peripheral_reference.deref();
        self.0.gpio_wp = pin.pin();
        self
    }

    pub fn set_int_pin(
        mut self,
        pin: impl peripheral::Peripheral<P = impl gpio::InputPin>,
    ) -> Self {
        let peripheral_reference = pin.into_ref();
        let pin = peripheral_reference.deref();
        self.0.gpio_int = pin.pin();
        self
    }

    pub fn build(self) -> Result<SpiDevice, esp_err_t> {
        SpiDevice::initialize(self.0)
    }

    pub fn get_inner_configuration(&self) -> &sdspi_device_config_t {
        &self.0
    }
}

pub struct SpiDevice {
    configuration: sdspi_device_config_t,
    handle: sdspi_dev_handle_t,
}

impl Drop for SpiDevice {
    fn drop(&mut self) {
        let result = unsafe { sdspi_host_remove_device(self.handle) };

        if result != ESP_OK {
            panic!("Failed to remove SPI device");
        }
    }
}

impl SpiDevice {
    pub fn builder() -> SpiDeviceBuilder {
        SpiDeviceBuilder::default()
    }

    pub fn initialize_host() -> Result<(), esp_err_t> {
        let result = unsafe { sdspi_host_init() };

        if result != ESP_OK {
            Err(result)
        } else {
            Ok(())
        }
    }

    pub fn initialize(configuration: sdspi_device_config_t) -> Result<Self, esp_err_t> {
        let mut handle: sdspi_dev_handle_t = 0;

        let result = unsafe {
            sdspi_host_init_device(&configuration as *const sdspi_device_config_t, &mut handle)
        };

        if result == ESP_OK {
            Ok(Self {
                configuration,
                handle,
            })
        } else {
            Err(result)
        }
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
