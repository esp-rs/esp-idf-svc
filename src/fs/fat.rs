#[cfg(esp_idf_soc_sdmmc_host_supported)]
use core::ffi::c_void;

use crate::private::cstr::*;

use crate::sd::host::SdDevice;
use crate::{sd::host::SdHost, sys::*};

use super::config::Configuration;

use core::borrow::BorrowMut;

use esp_idf_hal::spi::SpiDriver;

pub struct Fat {
    base_path: CString,
    card: *mut sdmmc_card_t,
}

impl Drop for Fat {
    fn drop(&mut self) {
        unsafe {
            esp!(esp_vfs_fat_sdcard_unmount(
                self.base_path.as_ptr(),
                self.card
            ))
            .unwrap();
        }
    }
}

impl Fat {
    pub fn mount<'d, T>(
        config: Configuration,
        host: &'d SdHost<'d, T>,
        base_path: &str,
    ) -> Result<Self, EspError>
    where
        T: BorrowMut<SpiDriver<'d>> + 'd,
    {
        let mut card: *mut sdmmc_card_t = core::ptr::null_mut();

        let base_path = CString::new(base_path).unwrap();

        let config: esp_vfs_fat_mount_config_t = config.into();

        esp!(match host.get_device() {
            #[cfg(esp_idf_soc_sdmmc_host_supported)]
            SdDevice::Mmc(slot_configuration) => unsafe {
                esp_vfs_fat_sdmmc_mount(
                    base_path.as_ptr(),
                    host.get_inner_handle() as *const sdmmc_host_t,
                    slot_configuration.get_inner() as *const sdmmc_slot_config_t as *const c_void,
                    &config as *const esp_vfs_fat_mount_config_t,
                    &mut card as *mut *mut sdmmc_card_t,
                )
            },
            SdDevice::Spi(spi_device) => unsafe {
                esp_vfs_fat_sdspi_mount(
                    base_path.as_ptr(),
                    host.get_inner_handle() as *const sdmmc_host_t,
                    spi_device.get_device_configuration() as *const sdspi_device_config_t,
                    &config as *const esp_vfs_fat_mount_config_t,
                    &mut card as *mut *mut sdmmc_card_t,
                )
            },
        })?;

        Ok(Self { base_path, card })
    }
}
