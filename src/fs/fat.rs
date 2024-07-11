use core::borrow::Borrow;
#[cfg(esp_idf_soc_sdmmc_host_supported)]
use core::ffi::c_void;

use esp_idf_hal::spi::SpiDriver;

#[cfg(esp_idf_soc_sdmmc_host_supported)]
use crate::sd::mmc::SlotConfiguration;

use crate::private::cstr::*;

use crate::sd::spi::SpiDevice;
use crate::{sd::host::SdHost, sys::*};

use super::config::Configuration;

/// FAT filesystem.
///
/// This struct is used to mount any FAT filesystem on the VFS.
/// Since the VFS is bind to newlib, you can use the standard `std::fs` module to interact with the filesystem.
/// Once dropped, the filesystem will be unmounted.
pub struct Fat<T> {
    base_path: CString,
    card: *mut sdmmc_card_t,
    _host: SdHost<T>,
}

impl<T> Drop for Fat<T> {
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

#[cfg(esp_idf_soc_sdmmc_host_supported)]
impl<'d> Fat<SlotConfiguration<'d>> {
    /// Mounts the FAT filesystem present on the SD card using the SDMMC interface.
    pub fn mount_sdmmc(
        config: Configuration,
        host: SdHost<SlotConfiguration<'d>>,
        base_path: &str,
    ) -> Result<Self, EspError> {
        let mut card: *mut sdmmc_card_t = core::ptr::null_mut();

        let base_path = CString::new(base_path).unwrap();

        let config: esp_vfs_fat_mount_config_t = config.into();

        esp!(unsafe {
            esp_vfs_fat_sdmmc_mount(
                base_path.as_ptr(),
                host.get_inner_handle() as *const sdmmc_host_t,
                host.get_device().get_inner() as *const sdmmc_slot_config_t as *const c_void,
                &config as *const esp_vfs_fat_mount_config_t,
                &mut card as *mut *mut sdmmc_card_t,
            )
        })?;

        Ok(Self {
            base_path,
            card,
            _host: host,
        })
    }
}

impl<'d, T> Fat<SpiDevice<'d, T>>
where
    T: Borrow<SpiDriver<'d>>,
{
    /// Mounts the FAT filesystem present on the SD card using the SPI interface.
    pub fn mount_spi(
        config: Configuration,
        host: SdHost<SpiDevice<'d, T>>,
        base_path: &str,
    ) -> Result<Self, EspError> {
        let mut card: *mut sdmmc_card_t = core::ptr::null_mut();

        let base_path = CString::new(base_path).unwrap();

        let config: esp_vfs_fat_mount_config_t = config.into();

        esp!(unsafe {
            esp_vfs_fat_sdspi_mount(
                base_path.as_ptr(),
                host.get_inner_handle() as *const sdmmc_host_t,
                host.get_device().get_device_configuration() as *const sdspi_device_config_t,
                &config as *const esp_vfs_fat_mount_config_t,
                &mut card as *mut *mut sdmmc_card_t,
            )
        })?;

        Ok(Self {
            base_path,
            card,
            _host: host,
        })
    }
}
