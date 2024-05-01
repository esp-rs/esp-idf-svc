#![allow(unsued_variables)]

use crate::sys::*;

use super::spi::SpiDevice;

pub struct SdHost(sdmmc_host_t);

const _HOST_FLAG_SPI: u32 = 1 << 3;
const _HOST_FLAG_DEINIT_ARG: u32 = 1 << 5;
const _DEFAULT_FREQUENCY: i32 = 20000;

const _DEFAULT_IO_VOLTAGE: f32 = 3.3;
const _SDMMC_HOST_FLAG_1BIT: u32 = 1 << 0;
const _SDMMC_HOST_FLAG_4BIT: u32 = 1 << 1;
const _SDMMC_HOST_FLAG_8BIT: u32 = 1 << 2;
const _SDMMC_HOST_FLAG_DDR: u32 = 1 << 3;
const _SDMMC_HOST_SLOT_0: u32 = 0;
const _SDMMC_HOST_SLOT_1: u32 = 1;
const _SDMMC_HOST_DEFAULT_SLOT: u32 = _SDMMC_HOST_SLOT_1;

impl SdHost {
    pub fn new_with_spi(device: &SpiDevice) -> Self {
        let host = sdmmc_host_t {
            flags: _HOST_FLAG_SPI | _HOST_FLAG_DEINIT_ARG,
            slot: *device.get_inner_handle(),
            max_freq_khz: _DEFAULT_FREQUENCY,
            io_voltage: 3.3,
            init: Some(sdspi_host_init),
            set_bus_width: None,
            get_bus_width: None,
            set_bus_ddr_mode: None,
            set_card_clk: Some(sdspi_host_set_card_clk),
            set_cclk_always_on: None,
            do_transaction: Some(sdspi_host_do_transaction),
            __bindgen_anon_1: sdmmc_host_t__bindgen_ty_1 {
                deinit_p: Some(sdspi_host_remove_device),
            },
            io_int_enable: Some(sdspi_host_io_int_enable),
            io_int_wait: Some(sdspi_host_io_int_wait),
            get_real_freq: Some(sdspi_host_get_real_freq),
            #[cfg(esp_idf_version = "5.2")]
            input_delay_phase: sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_0,
            #[cfg(esp_idf_version = "5.2")]
            set_input_delay: None,
            command_timeout_ms: 0,
        };

        Self(host)
    }

    /// Create a new SD/MMC host with the default configuration.
    /// This host will use the MMC slot 1, with 4-bit mode enabled, and max frequency set to 20MHz
    #[cfg(esp_idf_soc_sdmmc_host_supported)]
    pub fn new_with_mmc() -> Self {
        let host = sdmmc_host_t {
            flags: _SDMMC_HOST_FLAG_8BIT
                | _SDMMC_HOST_FLAG_4BIT
                | _SDMMC_HOST_FLAG_1BIT
                | _SDMMC_HOST_FLAG_DDR,
            slot: _SDMMC_HOST_DEFAULT_SLOT as i32,
            max_freq_khz: _DEFAULT_FREQUENCY,
            io_voltage: _DEFAULT_IO_VOLTAGE,
            init: Some(sdmmc_host_init),
            set_bus_width: Some(sdmmc_host_set_bus_width),
            get_bus_width: Some(sdmmc_host_get_slot_width),
            set_bus_ddr_mode: Some(sdmmc_host_set_bus_ddr_mode),
            set_card_clk: Some(sdmmc_host_set_card_clk),
            set_cclk_always_on: Some(sdmmc_host_set_cclk_always_on),
            do_transaction: Some(sdmmc_host_do_transaction),
            __bindgen_anon_1: sdmmc_host_t__bindgen_ty_1 {
                deinit: Some(sdmmc_host_deinit),
            },
            io_int_enable: Some(sdmmc_host_io_int_enable),
            io_int_wait: Some(sdmmc_host_io_int_wait),
            get_real_freq: Some(sdmmc_host_get_real_freq),
            #[cfg(esp_idf_version = "5.2")]
            input_delay_phase: sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_0,
            #[cfg(esp_idf_version = "5.2")]
            set_input_delay: Some(sdmmc_host_set_input_delay),
            command_timeout_ms: 0,
        };

        Self(host)
    }

    pub fn set_command_timeout(&mut self, timeout: u32) {
        self.0.command_timeout_ms = timeout as i32;
    }

    pub fn set_io_voltage(&mut self, voltage: f32) {
        self.0.io_voltage = voltage;
    }

    pub fn set_maximum_frequency(&mut self, frequency: i32) {
        self.0.max_freq_khz = frequency;
    }

    pub fn get_inner_handle(&self) -> &sdmmc_host_t {
        &self.0
    }
}
