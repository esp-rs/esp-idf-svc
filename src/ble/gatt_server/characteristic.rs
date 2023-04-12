use core::fmt::Formatter;

use std::sync::{Arc, Mutex, RwLock};

use ::log::{debug, warn};

use esp_idf_sys::{
    esp_attr_control_t, esp_attr_value_t, esp_ble_gatts_add_char,
    esp_ble_gatts_cb_param_t_gatts_read_evt_param, esp_ble_gatts_cb_param_t_gatts_write_evt_param,
    esp_ble_gatts_set_attr_value, esp_nofail,
};

use crate::{
    ble::gatt_server::descriptor::Descriptor,
    ble::utilities::{AttributeControl, AttributePermissions, BleUuid, CharacteristicProperties},
    nvs::EspDefaultNvs,
};

type WriteCallback = dyn Fn(Vec<u8>, esp_ble_gatts_cb_param_t_gatts_write_evt_param) + Send + Sync;

/// Represents a GATT characteristic.
#[derive(Clone)]
pub struct Characteristic {
    /// The name of the characteristic, for debugging purposes.
    name: Option<String>,
    /// The characteristic identifier.
    pub(crate) uuid: BleUuid,
    /// The function to be called when a write happens. This functions receives the written value in the first parameter, a `Vec<u8>`.
    pub(crate) write_callback: Option<Arc<WriteCallback>>,
    /// A list of descriptors for this characteristic.
    pub(crate) descriptors: Vec<Arc<RwLock<Descriptor>>>,
    /// The handle that the Bluetooth stack assigned to this characteristic.
    pub(crate) attribute_handle: Option<u16>,
    /// The handle of the containing service.
    service_handle: Option<u16>,
    /// The access permissions for this characteristic.
    permissions: AttributePermissions,
    /// The properties that are announced for this characteristic.
    pub(crate) properties: CharacteristicProperties,
    /// The way this characteristic is read.
    pub(crate) control: AttributeControl,
    /// A buffer for keeping in memory the actual value of this characteristic.
    pub(crate) internal_value: Vec<u8>,
    /// The maximum length of the characteristic value.
    max_value_length: Option<u16>,
    /// A copy of the `control` property, in the `esp_attr_control_t` type, passed directly to the Bluetooth stack.
    internal_control: esp_attr_control_t,
    /// Nvs storage used by Client Characteristic Configuration Descriptor (CCCD)
    nvs_storage: Option<Arc<Mutex<EspDefaultNvs>>>,
}

impl Characteristic {
    /// Creates a new [`Characteristic`].
    #[must_use]
    pub fn new(uuid: BleUuid) -> Self {
        Self {
            name: None,
            uuid,
            internal_value: vec![0],
            write_callback: None,
            descriptors: Vec::new(),
            attribute_handle: None,
            service_handle: None,
            permissions: AttributePermissions::default(),
            properties: CharacteristicProperties::default(),
            control: AttributeControl::AutomaticResponse(vec![0]),
            internal_control: AttributeControl::AutomaticResponse(vec![0]).into(),
            max_value_length: None,
            nvs_storage: None,
        }
    }

    /// Adds a [`Descriptor`] to the [`Characteristic`].
    pub fn descriptor(&mut self, descriptor: &Arc<RwLock<Descriptor>>) -> &mut Self {
        self.descriptors.push(descriptor.clone());
        self
    }

    /// Sets the name of the [`Characteristic`].
    ///
    /// This name is only used for debugging purposes.
    pub fn name<S: Into<String>>(&mut self, name: S) -> &mut Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the access permissions for this [`Characteristic`].
    pub fn permissions(&mut self, permissions: AttributePermissions) -> &mut Self {
        self.permissions = permissions;
        self
    }

    /// Sets the properties for this [`Characteristic`].
    pub fn properties(&mut self, properties: CharacteristicProperties) -> &mut Self {
        self.properties = properties;
        self
    }

    /// Sets the maximum length for the content of this characteristic. The default value is 8 bytes.
    pub fn max_value_length(&mut self, length: u16) -> &mut Self {
        self.max_value_length = Some(length);
        self
    }

    /// Sets the nvs storage of the [`Characteristic`].
    pub fn set_nvs_storage(&mut self, nvs_storage: Option<Arc<Mutex<EspDefaultNvs>>>) -> &mut Self {
        self.nvs_storage = nvs_storage;
        self
    }

    /// Sets the read callback for this characteristic.
    /// The callback will be called when a client reads the value of this characteristic.
    ///
    /// The callback must return a `Vec<u8>` containing the value to be put into the response to the read request.
    ///
    /// # Notes
    ///
    /// The callback will be called from the Bluetooth stack's context, so it must not block.
    pub fn on_read<
        C: Fn(esp_ble_gatts_cb_param_t_gatts_read_evt_param) -> Vec<u8> + Send + Sync + 'static,
    >(
        &mut self,
        callback: C,
    ) -> &mut Self {
        if !self.properties.read || !self.permissions.read_access {
            warn!(
                "Characteristic {} does not have read permissions. Ignoring read callback.",
                self
            );

            return self;
        }

        self.control = AttributeControl::ResponseByApp(Arc::new(callback));
        self.internal_control = self.control.clone().into();

        self
    }

    /// Sets the write callback for this characteristic.
    /// The callback will be called when a client writes to this characteristic.
    ///
    /// The callback receives a `Vec<u8>` with the written value.
    /// It is up to the library user to decode the data into a meaningful format.
    pub fn on_write(
        &mut self,
        callback: impl Fn(Vec<u8>, esp_ble_gatts_cb_param_t_gatts_write_evt_param)
            + Send
            + Sync
            + 'static,
    ) -> &mut Self {
        if !((self.properties.write || self.properties.write_without_response)
            && self.permissions.write_access)
        {
            warn!(
                "Characteristic {} does not have write permissions. Ignoring write callback.",
                self
            );

            return self;
        }

        self.write_callback = Some(Arc::new(callback));
        self
    }

    /// Creates a new "User description" descriptor for this characteristic
    /// that contains the name of the characteristic.
    pub fn show_name(&mut self) -> &mut Self {
        if let Some(name) = self.name.clone() {
            self.descriptor(&Arc::new(RwLock::new(Descriptor::user_description(name))));
        }

        if let BleUuid::Uuid16(_) = self.uuid {
            warn!("You're specifying a user description for a standard characteristic. This might be useless.");
        }

        self
    }

    /// Sets the value of this [`Characteristic`].
    ///
    /// Sends notifications and indications to all subscribed clients.
    ///
    /// # Panics
    ///
    /// Panics if the value is too long and the characteristic is already registered.
    ///
    /// # Notes
    ///
    /// Before starting the server, you can freely set the value of a characteristic.
    /// The maximum value length will be derived from the length of the initial value.
    /// If you plan to expose only one data type for all the lifetime of the characteristic,
    /// then you'll never need to use the [`Self.value_length`] method, because
    /// the maximum size will be automatically set to the length of the latest value
    /// set before starting the server.
    pub fn set_value<T: Into<Vec<u8>>>(&mut self, value: T) -> &mut Self {
        let value: Vec<u8> = value.into();

        #[allow(clippy::manual_assert)]
        if let Some(max_value_length) = self.max_value_length {
            if value.len() > max_value_length as usize {
                panic!(
                    "Value is too long for characteristic {}. The explicitly set maximum length is {max_value_length} bytes.",self
                );
            }
        } else if self.attribute_handle.is_some() && value.len() > self.internal_value.len() {
            panic!(
                "Value is too long for characteristic {}. The implicitly set maximum length is {} bytes.",
                self,
                self.internal_value.len()
            );
        }

        self.internal_value = value;
        self.control = AttributeControl::AutomaticResponse(self.internal_value.clone());
        self.internal_control = self.control.clone().into();

        debug!(
            "Trying to set value of {} to {:02X?}.",
            self, self.internal_value
        );

        if let Some(handle) = self.attribute_handle {
            #[allow(clippy::cast_possible_truncation)]
            unsafe {
                esp_nofail!(esp_ble_gatts_set_attr_value(
                    handle,
                    self.internal_value.len() as u16,
                    self.internal_value.as_slice().as_ptr()
                ));
            }
        }

        self
    }

    /// Returns a reference to the built [`Characteristic`] behind an `Arc` and an `RwLock`.
    ///
    /// The returned value can be passed to any function of this crate that expects a [`Characteristic`].
    /// It can be used in different threads, because it is protected by an `RwLock`.
    #[must_use]
    pub fn build(&self) -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(self.clone()))
    }

    /// Registers the [`Characteristic`] at the given service handle.
    pub(crate) fn register_self(&mut self, service_handle: u16) {
        debug!(
            "Registering {} into service at handle 0x{:04x}.",
            self, service_handle
        );
        self.service_handle = Some(service_handle);

        #[allow(clippy::manual_assert)]
        if let AttributeControl::AutomaticResponse(_) = self.control {
            if self.internal_value.is_empty() {
                panic!("Automatic response requires a value to be set.");
            }
        }

        // Register a CCCD if needed.
        if self.properties.notify || self.properties.indicate {
            self.descriptor(&Descriptor::cccd(self.nvs_storage.clone()).build());
        }

        #[allow(clippy::cast_possible_truncation)]
        unsafe {
            esp_nofail!(esp_ble_gatts_add_char(
                service_handle,
                &mut self.uuid.into(),
                self.permissions.into(),
                self.properties.into(),
                &mut esp_attr_value_t {
                    attr_max_len: self
                        .max_value_length
                        .unwrap_or(self.internal_value.len() as u16),
                    attr_len: self.internal_value.len() as u16,
                    attr_value: self.internal_value.as_mut_slice().as_mut_ptr(),
                },
                &mut self.internal_control,
            ));
        }
    }

    /// Registers the descriptors of this [`Characteristic`].
    ///
    /// This function should be called on the event of the characteristic being registered.
    ///
    /// # Panics
    ///
    /// Panics if the service handle is not registered.
    ///
    /// # Notes
    ///
    /// Bluedroid does not offer a way to register descriptors to a specific characteristic.
    /// This is simply done by registering the characteristic and then registering its descriptors.
    pub(crate) fn register_descriptors(&mut self) {
        debug!("Registering {}'s descriptors.", &self);
        let service_handle = self
            .service_handle
            .expect("Cannot register a descriptor to a characteristic without a service handle.");
        self.descriptors.iter_mut().for_each(|descriptor| {
            descriptor.write().unwrap().register_self(service_handle);
        });
    }

    pub(crate) fn get_cccd_status(
        &self,
        param: esp_ble_gatts_cb_param_t_gatts_read_evt_param,
    ) -> Option<(bool, bool)> {
        if let Some(cccd) = self
            .descriptors
            .iter()
            .find(|desc| desc.read().unwrap().uuid == BleUuid::Uuid16(0x2902))
        {
            if let AttributeControl::ResponseByApp(callback) = &cccd.read().unwrap().control {
                let value = callback(param);

                return Some((
                    value[0] & 0b0000_0001 == 0b0000_0001,
                    value[0] & 0b0000_0010 == 0b0000_0010,
                ));
            }
        }

        None
    }
}

impl std::fmt::Display for Characteristic {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({})",
            self.name
                .clone()
                .unwrap_or_else(|| "Unnamed characteristic".to_string()),
            self.uuid
        )
    }
}

impl std::fmt::Debug for Characteristic {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // Debug representation of a characteristic.
        f.debug_struct("Characteristic")
            .field("name", &self.name)
            .field("uuid", &self.uuid)
            .field("write_callback", &self.write_callback.is_some())
            .field("descriptors", &self.descriptors)
            .field("attribute_handle", &self.attribute_handle)
            .field("service_handle", &self.service_handle)
            .field("permissions", &self.permissions)
            .field("properties", &self.properties)
            .field("control", &self.control)
            .field("internal_value", &self.internal_value)
            .field("max_value_length", &self.max_value_length)
            .field("internal_control", &self.internal_control)
            .finish()
    }
}
