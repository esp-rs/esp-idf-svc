extern crate alloc;

use core::ops::Deref;

use ::log::*;
use mutex_trait::*;

use esp_idf_sys::c_types::*;
use esp_idf_sys::*;

use crate::private::cstr::*;

static mut DEFAULT_TAKEN: EspMutex<bool> = EspMutex::new(false);
static mut NONDEFAULT_LOCKED: EspMutex<alloc::collections::BTreeSet<CString>> =
    EspMutex::new(alloc::collections::BTreeSet::new());

#[derive(Debug)]
pub struct EspDefaultNvs(EspNvs);

impl EspDefaultNvs {
    pub fn new() -> Result<Self, EspError> {
        unsafe {
            DEFAULT_TAKEN.lock(|taken| {
                if *taken {
                    Err(EspError::from(ESP_ERR_INVALID_STATE as i32).unwrap())
                } else {
                    let part = crate::private::cstr::from_cstr(NVS_DEFAULT_PART_NAME);
                    let default_nvs = EspNvs::new(part)?;

                    *taken = true;
                    Ok(Self(default_nvs))
                }
            })
        }
    }
}

impl Deref for EspDefaultNvs {
    type Target = EspNvs;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for EspDefaultNvs {
    fn drop(&mut self) {
        unsafe {
            DEFAULT_TAKEN.lock(|taken| {
                *taken = false;
            });
        }

        info!("Dropped");
    }
}

#[derive(Debug)]
pub struct EspNvs(CString);

impl EspNvs {
    pub fn new(partition: impl AsRef<str>) -> Result<Self, EspError> {
        unsafe { NONDEFAULT_LOCKED.lock(|registrations| Self::init(partition, registrations)) }
    }

    fn init(
        partition: impl AsRef<str>,
        registrations: &mut alloc::collections::BTreeSet<CString>,
    ) -> Result<Self, EspError> {
        info!("initializing nvs partition {:?}", partition.as_ref());
        let c_partition = CString::new(partition.as_ref()).unwrap();

        if registrations.contains(c_partition.as_ref()) {
            return Err(EspError::from(ESP_ERR_INVALID_STATE as i32).unwrap());
        }

        unsafe {
            if let Some(err) = EspError::from(nvs_flash_init_partition(c_partition.as_ptr())) {
                match err.code() as u32 {
                    ESP_ERR_NVS_NO_FREE_PAGES | ESP_ERR_NVS_NEW_VERSION_FOUND => {
                        esp!(nvs_flash_erase_partition(c_partition.as_ptr()))?;
                        esp!(nvs_flash_init_partition(c_partition.as_ptr()))?;
                    }
                    _ => (),
                }
            }
        }

        registrations.insert(c_partition.clone());

        Ok(Self(c_partition))
    }

    pub fn open(&self, namespace: impl AsRef<str>, read_write: bool) -> Result<NvsHandle, EspError> {
        let c_namespace = CString::new(namespace.as_ref()).unwrap();

        let mut raw_handle: nvs_handle_t = 0;
        unsafe {
            esp!(nvs_open_from_partition(
                self.0.as_ptr(),
                c_namespace.as_ptr(),
                if read_write {
                    nvs_open_mode_t_NVS_READWRITE
                } else {
                    nvs_open_mode_t_NVS_READONLY
                },
                &mut raw_handle,
            ))?;
        }

        Ok(NvsHandle {
            namespace: c_namespace,
            handle: raw_handle,
        })
    }
}

impl Drop for EspNvs {
    fn drop(&mut self) {
        unsafe {
            NONDEFAULT_LOCKED.lock(|registrations| {
                esp!(nvs_flash_deinit_partition(self.0.as_ptr())).unwrap();
                registrations.remove(self.0.as_ref());
            });
        }

        info!("dropped nvs (deinited partition)");
    }
}

pub struct NvsHandle {
    namespace: CString,
    handle: nvs_handle_t,
}

impl NvsHandle {
    pub fn set_i8(&mut self, key: impl AsRef<str>, value: i8) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        unsafe { esp!(nvs_set_i8(self.handle, c_key.as_ptr(), value)) }
    }

    pub fn get_i8(&self, key: impl AsRef<str>) -> Result<Option<i8>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let mut out = 0;
        match unsafe { esp!(nvs_get_i8(self.handle, c_key.as_ptr(), &mut out,)) } {
            Ok(_) => Ok(Some(out)),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_u8(&mut self, key: impl AsRef<str>, value: u8) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        unsafe { esp!(nvs_set_u8(self.handle, c_key.as_ptr(), value)) }
    }

    pub fn get_u8(&self, key: impl AsRef<str>) -> Result<Option<u8>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let mut out = 0;
        match unsafe { esp!(nvs_get_u8(self.handle, c_key.as_ptr(), &mut out,)) } {
            Ok(_) => Ok(Some(out)),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_i16(&mut self, key: impl AsRef<str>, value: i16) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        unsafe { esp!(nvs_set_i16(self.handle, c_key.as_ptr(), value)) }
    }

    pub fn get_i16(&self, key: impl AsRef<str>) -> Result<Option<i16>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let mut out = 0;
        match unsafe { esp!(nvs_get_i16(self.handle, c_key.as_ptr(), &mut out,)) } {
            Ok(_) => Ok(Some(out)),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_u16(&mut self, key: impl AsRef<str>, value: u16) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        unsafe { esp!(nvs_set_u16(self.handle, c_key.as_ptr(), value)) }
    }

    pub fn get_u16(&self, key: impl AsRef<str>) -> Result<Option<u16>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let mut out = 0;
        match unsafe { esp!(nvs_get_u16(self.handle, c_key.as_ptr(), &mut out,)) } {
            Ok(_) => Ok(Some(out)),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_i32(&mut self, key: impl AsRef<str>, value: i32) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        unsafe { esp!(nvs_set_i32(self.handle, c_key.as_ptr(), value)) }
    }

    pub fn get_i32(&self, key: impl AsRef<str>) -> Result<Option<i32>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let mut out = 0;
        match unsafe { esp!(nvs_get_i32(self.handle, c_key.as_ptr(), &mut out,)) } {
            Ok(_) => Ok(Some(out)),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_u32(&mut self, key: impl AsRef<str>, value: u32) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        unsafe { esp!(nvs_set_u32(self.handle, c_key.as_ptr(), value)) }
    }

    pub fn get_u32(&self, key: impl AsRef<str>) -> Result<Option<u32>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let mut out = 0;
        match unsafe { esp!(nvs_get_u32(self.handle, c_key.as_ptr(), &mut out,)) } {
            Ok(_) => Ok(Some(out)),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_i64(&mut self, key: impl AsRef<str>, value: i64) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        unsafe { esp!(nvs_set_i64(self.handle, c_key.as_ptr(), value)) }
    }

    pub fn get_i64(&self, key: impl AsRef<str>) -> Result<Option<i64>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let mut out = 0;
        match unsafe { esp!(nvs_get_i64(self.handle, c_key.as_ptr(), &mut out,)) } {
            Ok(_) => Ok(Some(out)),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_u64(&mut self, key: impl AsRef<str>, value: u64) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        unsafe { esp!(nvs_set_u64(self.handle, c_key.as_ptr(), value)) }
    }

    pub fn get_u64(&self, key: impl AsRef<str>) -> Result<Option<u64>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let mut out = 0;
        match unsafe { esp!(nvs_get_u64(self.handle, c_key.as_ptr(), &mut out,)) } {
            Ok(_) => Ok(Some(out)),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_str(&mut self, key: impl AsRef<str>, value: &str) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let c_str = CString::new(value).unwrap();
        unsafe { esp!(nvs_set_str(self.handle, c_key.as_ptr(), c_str.as_ptr(),)) }
    }

    pub fn get_str(&self, key: impl AsRef<str>) -> Result<Option<String>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();

        // first call with a null data pointer to get the size of the stored blob
        let mut required_size: u32 = 0;
        match unsafe {
            esp!(nvs_get_str(
                self.handle,
                c_key.as_ptr(),
                core::ptr::null_mut() as *mut c_char,
                &mut required_size,
            ))
        } {
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => return Ok(None),
            Err(e) => return Err(e),
            _ => {}
        }

        // allocate with the right size and read the blob
        let mut data: Vec<u8> = Vec::with_capacity(required_size as usize);
        unsafe {
            esp!(nvs_get_str(
                self.handle,
                c_key.as_ptr(),
                data.as_mut_ptr() as *mut c_char,
                &mut required_size,
            ))?;
            data.set_len(required_size as usize);
        }

        let out_string = CString::new(data).unwrap();
        Ok(Some(out_string.into_string().unwrap()))
    }

    pub fn set_blob(&mut self, key: impl AsRef<str>, value: impl AsRef<[u8]>) -> Result<(), EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        let value = value.as_ref();
        unsafe {
            esp!(nvs_set_blob(
                self.handle,
                c_key.as_ptr(),
                value.as_ptr() as *const c_void,
                value.len() as u32
            ))
        }
    }

    pub fn get_blob(&self, key: impl AsRef<str>) -> Result<Option<Vec<u8>>, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();

        // first call with a null data pointer to get the size of the stored blob
        let mut required_size: u32 = 0;
        match unsafe {
            esp!(nvs_get_blob(
                self.handle,
                c_key.as_ptr(),
                core::ptr::null_mut() as *mut c_void,
                &mut required_size,
            ))
        } {
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => return Ok(None),
            Err(e) => return Err(e),
            _ => {}
        }

        // allocate with the right size and read the blob
        let mut data: Vec<u8> = Vec::with_capacity(required_size as usize);
        unsafe {
            esp!(nvs_get_blob(
                self.handle,
                c_key.as_ptr(),
                data.as_mut_ptr() as *mut c_void,
                &mut required_size,
            ))?;
            data.set_len(required_size as usize);
        }

        Ok(Some(data))
    }

    /// erase a key in nvs, returns true if it was deleted, false if it was not found
    pub fn erase_key(&mut self, key: impl AsRef<str>) -> Result<bool, EspError> {
        let c_key = CString::new(key.as_ref()).unwrap();
        match unsafe { esp!(nvs_erase_key(self.handle, c_key.as_ptr())) } {
            Ok(_) => Ok(true),
            Err(e) if e.code() as u32 == ESP_ERR_NVS_NOT_FOUND => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn erase_all(&mut self) -> Result<(), EspError> {
        unsafe { esp!(nvs_erase_all(self.handle)) }
    }

    /// Write any pending changes to non-volatile storage.
    //
    // After setting any values, nvs_commit() must be called to ensure changes are written
    // to non-volatile storage. Individual implementations may write to storage at other times,
    // but this is not guaranteed.
    pub fn commit(&mut self) -> Result<(), EspError> {
        unsafe { esp!(nvs_commit(self.handle)) }
    }
}

impl Drop for NvsHandle {
    fn drop(&mut self) {
        unsafe { nvs_close(self.handle) }

        info!("dropped handle");
    }
}
