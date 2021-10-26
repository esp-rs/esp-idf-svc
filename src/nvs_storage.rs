use core::any::Any;

extern crate alloc;
use alloc::sync::Arc;
use alloc::vec;

use embedded_svc::storage::Storage;

use esp_idf_sys::*;

use crate::nvs::*;

pub struct EspNvsStorage(Arc<dyn Any>, NvsHandle);

impl EspNvsStorage {
    pub fn new_default(
        default_nvs: Arc<EspDefaultNvs>,
        namespace: impl AsRef<str>,
        read_write: bool,
    ) -> Result<Self, EspError> {
        let handle = default_nvs.open(namespace, read_write)?;

        Ok(Self(default_nvs, handle))
    }

    pub fn new(
        nvs: Arc<EspNvs>,
        namespace: impl AsRef<str>,
        read_write: bool,
    ) -> Result<Self, EspError> {
        let handle = nvs.open(namespace, read_write)?;

        Ok(Self(nvs, handle))
    }
}

impl Storage for EspNvsStorage {
    type Error = EspError;

    fn contains(&self, key: impl AsRef<str>) -> Result<bool, Self::Error> {
        match self.1.get_u64(key) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) if e.code() == ESP_ERR_NVS_INVALID_LENGTH as i32  => Ok(true),
            Err(e) => Err(e),
        }
    }

    fn remove(&mut self, key: impl AsRef<str>) -> Result<bool, Self::Error> {
        let res = self.1.erase_key(key)?;
        self.1.commit()?;

        Ok(res)
    }

    fn get_raw(&self, key: impl AsRef<str>) -> Result<Option<vec::Vec<u8>>, Self::Error> {
        let key = key.as_ref();

        match self.1.get_u64(key) {
            Ok(None) => Ok(None),
            Err(e) if e.code() == ESP_ERR_NVS_INVALID_LENGTH as i32 => self.1.get_blob(key),
            Ok(Some(mut value)) => {
                let len: u8 = (value & 0xff) as u8;
                value >>= 8;

                let array: [u8; 7] = [
                    (value & 0xff) as u8,
                    ((value >> 8) & 0xff) as u8,
                    ((value >> 16) & 0xff) as u8,
                    ((value >> 24) & 0xff) as u8,
                    ((value >> 32) & 0xff) as u8,
                    ((value >> 48) & 0xff) as u8,
                    ((value >> 56) & 0xff) as u8,
                ];

                Ok(Some(array[..len as usize].to_vec()))
            }
            Err(e) => Err(e)
        }
    }

    fn put_raw(
        &mut self,
        key: impl AsRef<str>,
        value: impl Into<vec::Vec<u8>>,
    ) -> Result<bool, Self::Error> {
        let key = key.as_ref();

        let (mut uvalue, small, found) = match self.1.get_u64(key) {
            Ok(None) => (0, false, false),
            Err(e) if e.code() == ESP_ERR_NVS_INVALID_LENGTH as i32  => (0, true, false),
            Ok(Some(v)) => (v, true, true),
            Err(e) => return Err(e)
        };

        let value = value.into();
        let new_small = value.len() < 8;

        if found && small != new_small {
            self.1.erase_key(key)?;
        }

        if new_small {
            for v in value.iter().rev() {
                uvalue <<= 8;
                uvalue |= *v as u_int64_t;
            }

            uvalue <<= 8;
            uvalue |= value.len() as u_int64_t;

            self.1.set_u64(key, uvalue)?;
        } else {
            self.1.set_blob(key, value)?;
        }

        self.1.commit()?;

        Ok(found)
    }
}
