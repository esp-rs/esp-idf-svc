use crate::sdspi::SdMmcCard;
use esp_idf_sys::c_types::c_void;
use esp_idf_sys::*;
use std::ffi::{CStr, CString};
use std::ptr;

pub struct FatFs {
    fs: *mut FATFS,
    base_path: CString, // Consider using Cow
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FatError(FRESULT);

impl FatFs {
    const FF_MAX_SS: usize = 4096; // https://github.com/espressif/esp-idf/blob/1d30c234556eb05e663ab6e0c13055455efb1052/components/fatfs/vfs/vfs_fat_sdmmc.c#L139

    pub fn register(
        base_path: &CStr,
        fat_drive: &CStr,
        max_files: usize,
    ) -> Result<Self, EspError> {
        let mut fs: *mut FATFS = ptr::null_mut();

        esp!(unsafe {
            esp_vfs_fat_register(
                base_path.as_ptr(),
                fat_drive.as_ptr(),
                max_files as _,
                &mut fs,
            )
        })?;

        Ok(FatFs {
            fs,
            base_path: base_path.to_owned(),
        })
    }

    fn fresult(result: FRESULT) -> Result<(), FatError> {
        if result == FRESULT_FR_OK {
            Ok(())
        } else {
            Err(FatError(result))
        }
    }

    pub fn mount(&mut self, path: &CStr, opt: u8) -> Result<(), FatError> {
        let result = unsafe { f_mount(self.fs, path.as_ptr(), opt) };
        Self::fresult(result)
    }

    pub fn unmount(path: &CStr) -> Result<(), FatError> {
        let result = unsafe { f_mount(ptr::null_mut(), path.as_ptr(), 0) };
        Self::fresult(result)
    }

    pub fn fdisk(pdrv: u8, ptbl: &[u32], work_area: &mut [u8]) -> Result<(), FatError> {
        let result = unsafe { f_fdisk(pdrv, ptbl.as_ptr(), work_area.as_mut_ptr() as *mut c_void) };
        Self::fresult(result)
    }

    // pub fn mkfs(path: &CStr, work_area: &[u8]) -> Result<(), FatError> {}

    pub fn diskio_get_drive() -> Result<u8, EspError> {
        let mut pdrv: u8 = FF_DRV_NOT_USED as u8;
        esp!(unsafe { ff_diskio_get_drive(&mut pdrv) })?;
        if pdrv == FF_DRV_NOT_USED as u8 {
            Err(EspError::from(ESP_ERR_NO_MEM).unwrap())
        } else {
            Ok(pdrv)
        }
    }

    pub fn diskio_register_sdmmc(pdrv: u8, card: &SdMmcCard) -> Result<(), EspError> {
        // esp!(unsafe { ff_diskio_register_sdmmc(pdrv, &card.0) })
        unimplemented!()
    }

    pub fn diskio_unregister(pdrv: u8) {
        // unsafe { ff_diskio_unregister(pdrv) }
        unsafe { ff_diskio_register(pdrv, ptr::null()) }
    }
}

impl Drop for FatFs {
    fn drop(&mut self) {
        esp!(unsafe { esp_vfs_fat_unregister_path(self.base_path.as_ptr()) }).unwrap();
    }
}
