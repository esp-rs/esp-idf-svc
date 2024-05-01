#[cfg(esp_idf_soc_sdmmc_host_supported)]
use std::os::raw::c_void;

use crate::private::cstr::*;

use crate::{
    sd::{host::SdHost, spi::SpiDevice},
    sys::*,
};

#[cfg(esp_idf_soc_sdmmc_host_supported)]
use crate::sd::mmc::SlotConfiguration;

pub struct FatBuilder {
    host: Option<SdHost>,
    spi_device: Option<SpiDevice>,
    #[cfg(esp_idf_soc_sdmmc_host_supported)]
    slot_configuration: Option<SlotConfiguration>,
    mount_config: esp_vfs_fat_mount_config_t,
    base_path: CString,
}

impl Default for FatBuilder {
    fn default() -> Self {
        Self {
            host: None,
            mount_config: esp_vfs_fat_mount_config_t {
                max_files: 4,
                format_if_mount_failed: false,
                allocation_unit_size: 16 * 1024,
                #[cfg(not(
                    esp_idf_version_major = "4",
                ))] // For ESP-IDF v5.0 and later
                disk_status_check_enable: false,
            },
            spi_device: None,
            #[cfg(esp_idf_soc_sdmmc_host_supported)]
            slot_configuration: None,
            base_path: to_cstring_arg("/").expect("Failed to create CString from /sdcard"),
        }
    }
}

impl FatBuilder {
    pub fn set_host(mut self, host: SdHost) -> Self {
        self.host = Some(host);
        self
    }

    pub fn set_spi_device(mut self, spi_device: SpiDevice) -> Self {
        #[cfg(esp_idf_soc_sdmmc_host_supported)]
        if self.slot_configuration.is_some() {
            panic!("SPI device cannot be set when using MMC slot configuration");
        }

        self.spi_device = Some(spi_device);
        self
    }

    #[cfg(esp_idf_soc_sdmmc_host_supported)]
    pub fn set_slot_configuration(mut self, slot_configuration: SlotConfiguration) -> Self {
        if self.spi_device.is_some() {
            panic!("Slot configuration cannot be set when using SPI device");
        }
        self.slot_configuration = Some(slot_configuration);
        self
    }

    pub fn set_max_files(mut self, max_files: u32) -> Self {
        self.mount_config.max_files = max_files as i32;
        self
    }

    pub fn set_format_if_mount_failed(mut self, format_if_mount_failed: bool) -> Self {
        self.mount_config.format_if_mount_failed = format_if_mount_failed;
        self
    }

    pub fn set_allocation_unit_size(mut self, allocation_unit_size: usize) -> Self {
        self.mount_config.allocation_unit_size = allocation_unit_size;
        self
    }

    pub fn set_base_path(mut self, base_path: &str) -> Self {
        self.base_path = CString::new(base_path).expect("Failed to create CString from base_path");
        self
    }

    pub fn build(self) -> Result<Fat, esp_err_t> {
        Fat::mount(self)
    }
}

pub struct Fat {
    builder: FatBuilder,
    card: *mut sdmmc_card_t,
}

impl Drop for Fat {
    fn drop(&mut self) {
        unsafe {
            esp_vfs_fat_sdcard_unmount(self.builder.base_path.as_ptr(), self.card);
        }
    }
}

impl Fat {
    pub fn builder() -> FatBuilder {
        FatBuilder::default()
    }

    pub fn mount(builder: FatBuilder) -> Result<Self, esp_err_t> {
        let mut card: *mut sdmmc_card_t = core::ptr::null_mut();

        let host = builder.host.as_ref().expect("Host not set");

        if let Some(spi_device) = &builder.spi_device {
            let result = unsafe {
                esp_vfs_fat_sdspi_mount(
                    builder.base_path.as_ptr(),
                    host.get_inner_handle() as *const sdmmc_host_t,
                    spi_device.get_device_configuration() as *const sdspi_device_config_t,
                    &builder.mount_config as *const esp_vfs_fat_mount_config_t,
                    &mut card as *mut *mut sdmmc_card_t,
                )
            };

            if result == ESP_OK {
                return Ok(Self { builder, card });
            } else {
                return Err(result);
            }
        }

        #[cfg(esp_idf_soc_sdmmc_host_supported)]
        if let Some(slot_configuration) = &builder.slot_configuration {
            let result = unsafe {
                esp_vfs_fat_sdmmc_mount(
                    builder.base_path.as_ptr(),
                    host.get_inner_handle() as *const sdmmc_host_t,
                    slot_configuration.get_inner() as *const sdmmc_slot_config_t as *const c_void,
                    &builder.mount_config as *const esp_vfs_fat_mount_config_t,
                    &mut card as *mut *mut sdmmc_card_t,
                )
            };

            if result == ESP_OK {
                return Ok(Self { builder, card });
            } else {
                return Err(result);
            }
        }

        panic!("Either SPI device or slot configuration must be set");
    }
}
