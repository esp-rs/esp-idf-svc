use alloc::fmt;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};

use ::log::{debug, info, warn};

use esp_idf_sys::{
    esp_attr_control_t, esp_attr_value_t, esp_ble_gatts_add_char_descr,
    esp_ble_gatts_cb_param_t_gatts_read_evt_param, esp_ble_gatts_cb_param_t_gatts_write_evt_param,
    esp_ble_gatts_set_attr_value, esp_nofail,
};

use crate::{
    ble::utilities::{AttributeControl, AttributePermissions, BleUuid},
    nvs::EspDefaultNvs,
};

type DescriptorWriteCallback =
    dyn Fn(Vec<u8>, esp_ble_gatts_cb_param_t_gatts_write_evt_param) + Send + Sync;

/// Represents a GATT descriptor.
#[derive(Clone)]
pub struct Descriptor {
    name: Option<String>,
    pub(crate) uuid: BleUuid,
    value: Vec<u8>,
    pub(crate) attribute_handle: Option<u16>,
    permissions: AttributePermissions,
    pub(crate) control: AttributeControl,
    internal_control: esp_attr_control_t,
    pub(crate) write_callback: Option<Arc<DescriptorWriteCallback>>,
    cccd_value: HashMap<String, Vec<u8>>,
}

impl Descriptor {
    /// Creates a new [`Descriptor`].
    #[must_use]
    pub fn new(uuid: BleUuid) -> Self {
        Self {
            name: None,
            uuid,
            value: vec![0],
            attribute_handle: None,
            permissions: AttributePermissions::default(),
            control: AttributeControl::AutomaticResponse(vec![0]),
            internal_control: AttributeControl::AutomaticResponse(vec![0]).into(),
            write_callback: None,
            cccd_value: HashMap::new(),
        }
    }

    /// Sets the name of the [`Descriptor`].
    ///
    /// This name is only used for debugging purposes.
    pub fn name(&mut self, name: &str) -> &mut Self {
        self.name = Some(String::from(name));
        self
    }

    /// Sets the permissions of the [`Descriptor`].
    pub fn permissions(&mut self, permissions: AttributePermissions) -> &mut Self {
        self.permissions = permissions;
        self
    }

    /// Sets the read callback for the [`Descriptor`].
    pub fn on_read<
        C: Fn(esp_ble_gatts_cb_param_t_gatts_read_evt_param) -> Vec<u8> + Send + Sync + 'static,
    >(
        &mut self,
        callback: C,
    ) -> &mut Self {
        if !self.permissions.read_access {
            warn!(
                "Descriptor {} does not have read permissions. Ignoring read callback.",
                self
            );

            return self;
        }

        self.control = AttributeControl::ResponseByApp(Arc::new(callback));
        self.internal_control = self.control.clone().into();

        self
    }

    /// Sets the write callback for the [`Descriptor`].
    pub fn on_write(
        &mut self,
        callback: impl Fn(Vec<u8>, esp_ble_gatts_cb_param_t_gatts_write_evt_param)
            + Send
            + Sync
            + 'static,
    ) -> &mut Self {
        if !self.permissions.write_access {
            warn!(
                "Descriptor {} does not have write permissions. Ignoring write callback.",
                self
            );

            return self;
        }

        self.write_callback = Some(Arc::new(callback));

        self
    }

    /// Sets the value of the [`Descriptor`].
    pub fn set_value<T: Into<Vec<u8>>>(&mut self, value: T) -> &mut Self {
        self.value = value.into();

        debug!("Trying to set value of {} to {:02X?}.", self, self.value);

        if let Some(handle) = self.attribute_handle {
            #[allow(clippy::cast_possible_truncation)]
            unsafe {
                esp_nofail!(esp_ble_gatts_set_attr_value(
                    handle,
                    self.value.len() as u16,
                    self.value.as_slice().as_ptr()
                ));
            }
        } else {
            info!(
                "Descriptor {} not registered yet, value will be set on registration.",
                self
            );
        }
        self
    }

    /// Gets the cccd value of the [`Descriptor`].
    pub fn get_cccd_value(&self, key: &str) -> Option<Vec<u8>> {
        self.cccd_value.get(key).cloned()
    }

    /// Sets the cccd value of the [`Descriptor`].
    pub fn set_cccd_value(&mut self, key: String, value: Vec<u8>) {
        dbg!(self.cccd_value.insert(key, value));
    }

    /// Returns a reference to the built [`Descriptor`] behind an `Arc` and an `RwLock`.
    ///
    /// The returned value can be passed to any function of this crate that expects a [`Descriptor`].
    /// It can be used in different threads, because it is protected by an `RwLock`.
    #[must_use]
    pub fn build(&self) -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(self.clone()))
    }
    pub(crate) fn register_self(&mut self, service_handle: u16) {
        debug!(
            "Registering {} into service at handle 0x{:04x}.",
            self, service_handle
        );

        #[allow(clippy::cast_possible_truncation)]
        unsafe {
            esp_nofail!(esp_ble_gatts_add_char_descr(
                service_handle,
                &mut self.uuid.into(),
                self.permissions.into(),
                &mut esp_attr_value_t {
                    attr_max_len: self.value.len() as u16,
                    attr_len: self.value.len() as u16,
                    attr_value: self.value.as_mut_slice().as_mut_ptr(),
                },
                &mut self.internal_control,
            ));
        }
    }

    /// Creates a new descriptor with the `0x2901` UUID, and the description string as its value.
    ///
    /// This is a common descriptor used to describe a characteristic to the user.
    /// See [`Characteristic::show_name`] for an easier way to assign this kind of descriptor to a [`Characteristic`].
    ///
    /// [`Characteristic::show_name`]: crate::gatt_server::Characteristic::show_name
    /// [`Characteristic`]: crate::gatt_server::Characteristic
    pub fn user_description<S: AsRef<str>>(description: S) -> Self {
        Self::new(BleUuid::from_uuid16(0x2901))
            .name("User Description")
            .permissions(AttributePermissions::new().read())
            .set_value(description.as_ref().as_bytes().to_vec())
            .clone()
    }

    /// Creates a CCCD.
    ///
    /// If the nvs_storage is set for the Characteristic
    /// The contents of the CCCD are stored in NVS and persisted across reboots.
    ///
    /// # Panics
    ///
    /// Panics if the NVS is not configured.
    #[must_use]
    pub fn cccd(nvs_storage: Option<Arc<Mutex<EspDefaultNvs>>>) -> Self {
        let descriptor = Arc::new(Mutex::new(Descriptor::new(BleUuid::from_uuid16(0x2902))));

        let descriptor_on_read = descriptor.clone();
        let storage_nvs_on_read = nvs_storage.clone();

        descriptor
            .lock()
            .unwrap()
            .name("Client Characteristic Configuration")
            .permissions(AttributePermissions::new().read().write())
            .on_read(
                move |param: esp_idf_sys::esp_ble_gatts_cb_param_t_gatts_read_evt_param| {
                    // Create a key from the connection address.
                    let key = format!(
                        "{:02X}{:02X}{:02X}{:02X}-{:04X}",
                        /* param.bda[1], */ param.bda[2],
                        param.bda[3],
                        param.bda[4],
                        param.bda[5],
                        param.handle
                    );

                    match &storage_nvs_on_read {
                        Some(storage) => {
                            // TODO: Find the characteristic that contains the handle.
                            // WARNING: Using the handle is incredibly stupid as the NVS is not erased across flashes.

                            // Prepare buffer and read correct CCCD value from non-volatile storage.
                            let mut buf: [u8; 2] = [0; 2];
                            if let Some(value) =
                                storage.lock().unwrap().get_raw(&key, &mut buf).unwrap()
                            {
                                debug!("Read CCCD value: {:?} for key {}.", value, key);
                                value.to_vec()
                            } else {
                                debug!("No CCCD value found for key {}.", key);
                                vec![0, 0]
                            }
                        }
                        None => match descriptor_on_read.lock().unwrap().get_cccd_value(&key) {
                            Some(value) => value,
                            None => vec![0, 0],
                        },
                    }
                },
            );

        descriptor
            .clone()
            .lock()
            .unwrap()
            .on_write(move |value, param| {
                // Create a key from the connection address.
                let key = format!(
                    "{:02X}{:02X}{:02X}{:02X}-{:04X}",
                    /* param.bda[1], */ param.bda[2],
                    param.bda[3],
                    param.bda[4],
                    param.bda[5],
                    param.handle
                );
                debug!("Write CCCD value: {:?} at key {}", value, key);
                match &nvs_storage {
                    Some(storage) => {
                        // Write CCCD value to non-volatile storage.
                        storage.lock().unwrap().set_raw(&key, &value).expect(
                            "Cannot put raw value to the NVS. Did you declare an NVS partition?",
                        );
                    }
                    None => {
                        descriptor
                            .clone()
                            .lock()
                            .unwrap()
                            .set_cccd_value(key, value);
                    }
                }
            })
            .clone()
    }
}

impl std::fmt::Display for Descriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({})",
            self.name
                .clone()
                .unwrap_or_else(|| "Unnamed descriptor".to_string()),
            self.uuid
        )
    }
}

impl std::fmt::Debug for Descriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        // Debug representation of a characteristic.
        f.debug_struct("Descriptor")
            .field("name", &self.name)
            .field("uuid", &self.uuid)
            .field("value", &self.value)
            .field("attribute_handle", &self.attribute_handle)
            .field("permissions", &self.permissions)
            .field("control", &self.control)
            .field("internal_control", &self.internal_control)
            .field("internal_control", &self.internal_control)
            .field("write_callback is_some={}", &self.write_callback.is_some())
            .field("cccd_value", &self.cccd_value)
            .finish()
    }
}
