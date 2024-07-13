use core::borrow::BorrowMut;

use alloc::boxed::Box;

use config::{FatFsType, FormatConfiguration};

use crate::hal::sd::SdCardDriver;
use crate::sys::*;

extern crate alloc;

pub mod config {
    use core::num::NonZeroU16;

    /// Number of File Allocation Tables copies to create when formatting the partition.
    #[derive(Default, Copy, Clone, Eq, PartialEq)]
    pub enum FatCopies {
        /// A single copy of the FAT table.
        #[default]
        Single,
        /// Two copies of the FAT table.
        Two,
    }

    impl FatCopies {
        pub(crate) const fn copies(&self) -> u8 {
            match self {
                Self::Single => 1,
                Self::Two => 2,
            }
        }
    }

    /// Type of FAT filesystem to create when formatting the partition.
    #[derive(Copy, Clone, Eq, PartialEq)]
    pub enum FatFsType {
        /// Automatically choose the best FAT type depending on volume and cluster size.
        Auto(FatCopies),
        /// FAT12 filesystem.
        Fat(FatCopies),
        /// FAT32 filesystem.
        Fat32(FatCopies),
        /// ExFAT filesystem.
        ExFat,
    }

    impl FatFsType {
        pub(crate) const fn copies(&self) -> u8 {
            match self {
                Self::Auto(c) => c.copies(),
                Self::Fat(c) => c.copies(),
                Self::Fat32(c) => c.copies(),
                Self::ExFat => 1,
            }
        }
    }

    /// Configuration for formatting a FAT partition.
    pub struct FormatConfiguration {
        /// Type of FAT filesystem to create.
        pub fs_type: FatFsType,
        /// Volume data alignment in number of sectors.
        pub volume_data_alignment: NonZeroU16,
        /// Number of root directory entries.
        pub root_dir_entries: NonZeroU16,
        /// Cluster size in bytes.
        pub cluster_size: u32,
    }

    impl FormatConfiguration {
        /// Create a new default configuration
        pub const fn new() -> Self {
            Self {
                fs_type: FatFsType::Auto(FatCopies::Single),
                volume_data_alignment: unsafe { NonZeroU16::new_unchecked(1) },
                root_dir_entries: unsafe { NonZeroU16::new_unchecked(512) },
                cluster_size: 4096,
            }
        }
    }

    impl Default for FormatConfiguration {
        fn default() -> Self {
            Self::new()
        }
    }
}

enum Partition<T> {
    SdCard(T),
    RawPartition,
}

/// Represents a mounted FAT filesystem instance that can be used to interact with the filesystem.
/// The filesystem is automatically unmounted when the instance is dropped.
///
/// The interaction happens via the native, unsafe FATFS library API (i.e. `crate::sys::f_open`, `crate::sys::f_read` and so on).
/// An alternative way to mount the filesystem is to use the VFS API, which is more high-level and abstracts the underlying filesystem.
pub struct MountedFatFs<'a, T> {
    fs: &'a mut FatFs<T>,
    fatfs: Box<FATFS>,
}

impl<'a, T> MountedFatFs<'a, T> {
    /// Get the underlying FATFS instance.
    pub fn fatfs(&self) -> &FATFS {
        &self.fatfs
    }

    // TODO: Add safe methods to interact with the filesystem
}

impl<'a, T> Drop for MountedFatFs<'a, T> {
    fn drop(&mut self) {
        let drive_path = self.fs.drive_path();

        unsafe {
            f_mount(core::ptr::null_mut(), drive_path.as_ptr(), 0);
        }
    }
}

/// Represents a FAT filesystem.
pub struct FatFs<T> {
    drive: u8,
    _partition: Partition<T>,
}

impl<T> FatFs<T> {
    /// Create a new FAT filesystem instance for a given SD card driver.
    ///
    /// # Arguments
    /// - Drive number to assign to the filesystem.
    /// - SD card driver instance.
    pub fn new_sdcard<H>(drive: u8, mut sd_card_driver: T) -> Result<Self, EspError>
    where
        T: BorrowMut<SdCardDriver<H>>,
    {
        unsafe {
            ff_diskio_register_sdmmc(
                drive,
                sd_card_driver.borrow_mut().card() as *const _ as *mut _,
            );
        }

        Ok(Self {
            drive,
            _partition: Partition::SdCard(sd_card_driver),
        })
    }

    /// Get the drive number of the filesystem.
    pub fn drive(&self) -> u8 {
        self.drive
    }

    /// Format the partition with the given configuration.
    ///
    /// # Arguments
    /// - Formatting configuration.
    /// - Buffer to use when formatting.
    pub fn format(
        &mut self,
        configuration: &FormatConfiguration,
        buf: &mut [u8],
    ) -> Result<(), EspError> {
        let drive_path = self.drive_path();

        let opt = MKFS_PARM {
            fmt: match configuration.fs_type {
                FatFsType::Auto(_) => FM_ANY,
                FatFsType::Fat(_) => FM_FAT,
                FatFsType::Fat32(_) => FM_FAT32,
                FatFsType::ExFat => FM_EXFAT,
            } as _,
            au_size: configuration.cluster_size,
            n_fat: configuration.fs_type.copies(),
            n_root: configuration.root_dir_entries.get() as _,
            align: configuration.volume_data_alignment.get() as _,
        };

        unsafe {
            f_mkfs(
                drive_path.as_ptr(),
                &opt,
                buf.as_mut_ptr() as *mut _,
                buf.len() as _,
            );
        }

        Ok(())
    }

    /// Mount the filesystem and return a handle to it.
    pub fn mount(&mut self) -> Result<MountedFatFs<'_, T>, EspError> {
        let mut fatfs: Box<FATFS> = Box::default(); // TODO: Large stack size

        let drive_path = self.drive_path();

        unsafe {
            f_mount(&mut *fatfs, drive_path.as_ptr(), 0);
        }

        Ok(MountedFatFs { fs: self, fatfs })
    }

    pub(crate) fn drive_path_from(drive: u8) -> [core::ffi::c_char; 2] {
        [drive as _, 0]
    }

    pub(crate) fn drive_path(&self) -> [core::ffi::c_char; 2] {
        Self::drive_path_from(self.drive)
    }
}

impl FatFs<()> {
    /// Create a new FAT filesystem instance for a given raw partition in the internal flash.
    /// This API is unsafe because current `esp-idf-hal` does not have a safe way to
    /// represent a flash partition.
    ///
    /// # Arguments
    /// - Drive number to assign to the filesystem.
    /// - Raw partition pointer.
    ///
    /// # Safety
    ///
    /// While the filesystem object is alive, the partition should not be used elsewhere
    pub unsafe fn new_raw_part(
        drive: u8,
        partition: *const esp_partition_t,
    ) -> Result<Self, EspError> {
        unsafe {
            ff_diskio_register_raw_partition(drive, partition);
        }

        Ok(Self {
            drive,
            _partition: Partition::RawPartition,
        })
    }
}

impl<T> Drop for FatFs<T> {
    fn drop(&mut self) {
        unsafe {
            ff_diskio_register(self.drive, core::ptr::null_mut());
        }
    }
}
