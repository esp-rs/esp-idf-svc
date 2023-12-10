use crate::sys::*;
use core::ops::Deref;
use std::sync::Mutex;

use super::SpiDevice;

pub struct SdHost(Mutex<sdmmc_host_t>);

const HOST_FLAG_SPI: u32 = 1 << 3;
const HOST_FLAG_DEINIT_ARG: u32 = 1 << 5;
const DEFAULT_FREQUENCY: i32 = 20000;

impl From<&mut SdHost> for sdmmc_host_t {
    fn from(host: &mut SdHost) -> Self {
        let inner = match host.0.lock() {
            Ok(inner) => inner,
            Err(_) => panic!("Failed to lock SPI host")
        };
        *inner.deref()
    }
}

impl SdHost {
    pub fn new_with_spi(device: &mut SpiDevice) -> Self {
        let mut host = sdmmc_host_t {
            flags: HOST_FLAG_SPI | HOST_FLAG_DEINIT_ARG,
            slot: device.into(),
            max_freq_khz: DEFAULT_FREQUENCY,
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
            command_timeout_ms: 0
        };
        Self(Mutex::new(host))
    }
}
