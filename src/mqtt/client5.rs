use std::vec::Vec;

#[allow(unused_imports)]
pub use super::*;

extern crate alloc;
use alloc::ffi::CString;

use embedded_svc::mqtt::client5::{UserPropertyItem, UserPropertyList};

#[allow(unused_imports)]
use esp_idf_hal::sys::*;

pub struct EspUserPropertyList(pub(crate) mqtt5_user_property_handle_t);

impl EspUserPropertyList {
    pub  fn from<'a>(items: &&[UserPropertyItem<'a>]) -> Self {
        let handle = mqtt5_user_property_handle_t::default();
        let mut list = EspUserPropertyList(handle);
        list.set_items(items)
            .expect("Failed to set user properties");

        Self(handle)
    }

    pub fn as_ptr(&self) -> mqtt5_user_property_handle_t {
        self.0
    }

    pub fn as_const_ptr(&self) -> *const mqtt5_user_property_handle_t {
        self.0 as *const mqtt5_user_property_handle_t
    }

    fn count(&self) -> u8 {
        let count = unsafe { esp_mqtt5_client_get_user_property_count(self.0) };
        count
    }

    fn set_items(&mut self, properties: &[UserPropertyItem]) -> Result<(), EspError> {
        let mut items: Vec<esp_mqtt5_user_property_item_t> = properties
            .iter()
            .map(|item| {
                let key_cstr = CString::new(item.key).unwrap();
                let value_cstr = CString::new(item.value).unwrap();

                let item = esp_mqtt5_user_property_item_t {
                    key: key_cstr.as_ptr(),
                    value: value_cstr.as_ptr(),
                };
                item
            })
            .collect();

        let error = unsafe {
            let items_ptr = items.as_mut_ptr();
            let result =
                esp_mqtt5_client_set_user_property(&mut self.0, items_ptr, items.len() as u8);
            result
        };
        esp!(error)?;
        Ok(())
    }

    fn get_items(&self) -> Result<Option<Vec<UserPropertyItem>>, EspError> {
        let count = unsafe { esp_mqtt5_client_get_user_property_count(self.0) };
        if count == 0 {
            return Ok(None);
        }
        let mut items: Vec<esp_mqtt5_user_property_item_t> = Vec::with_capacity(count as usize);
        items.resize_with(count as usize, || esp_mqtt5_user_property_item_t {
            key: core::ptr::null(),
            value: core::ptr::null(),
        });
        let error = unsafe {
            esp_mqtt5_client_get_user_property(
                self.0,
                items.as_mut_ptr(),
                &mut items.len() as *mut usize as *mut u8,
            )
        };
        esp!(error)?;
        let result: Vec<UserPropertyItem> = items.into_iter().map(to_user_property).collect();
        Ok(Some(result))
    }

    fn clear(&self) {
        unsafe {
            esp_mqtt5_client_delete_user_property(self.0);
        }
    }
}

fn to_user_property<'a>(item: esp_mqtt5_user_property_item_t) -> UserPropertyItem<'a> {
    let key = unsafe { core::ffi::CStr::from_ptr(item.key) }
        .to_str()
        .unwrap();
    let value = unsafe { core::ffi::CStr::from_ptr(item.value) }
        .to_str()
        .unwrap();
    UserPropertyItem { key, value }
}

impl UserPropertyList<EspError> for EspUserPropertyList {
    fn set_items(&mut self, properties: &[UserPropertyItem]) -> Result<(), EspError> {
        EspUserPropertyList::set_items(self, properties)
    }

    fn get_items(&self) -> Result<Option<Vec<UserPropertyItem>>, EspError> {
        EspUserPropertyList::get_items(self)
    }

    fn clear(&self) {
        EspUserPropertyList::clear(self)
    }

    fn count(&self) -> u8 {
        EspUserPropertyList::count(self)
    }
}

impl UserPropertyList<EspError> for &EspUserPropertyList {
    fn set_items(&mut self, properties: &[UserPropertyItem]) -> Result<(), EspError> {
        // SAFETY: The caller must guarantee exclusive access when calling set_items via &self.
        let mut_self = unsafe { &mut *(self as *const _ as *mut EspUserPropertyList) };
        EspUserPropertyList::set_items(mut_self, properties)
    }

    fn get_items(&self) -> Result<Option<Vec<UserPropertyItem>>, EspError> {
        EspUserPropertyList::get_items(self)
    }

    fn clear(&self) {
        EspUserPropertyList::clear(self)
    }

    fn count(&self) -> u8 {
        EspUserPropertyList::count(self)
    }
}
