use std::sync::{Arc, RwLock};

use crate::{
    leaky_box_raw,
    utilities::{AttributeControl, AttributePermissions, BleUuid},
};

use esp_idf_sys::{
    esp_attr_control_t, esp_attr_value_t, esp_ble_gatts_add_char_descr,
    esp_ble_gatts_cb_param_t_gatts_read_evt_param, esp_ble_gatts_cb_param_t_gatts_write_evt_param,
    esp_ble_gatts_set_attr_value, esp_nofail,
};
use log::{debug, info, warn};

/// Represents a GATT descriptor.
#[derive(Debug, Clone)]
pub struct Descriptor {
    name: Option<String>,
    pub(crate) uuid: BleUuid,
    value: Vec<u8>,
    pub(crate) attribute_handle: Option<u16>,
    permissions: AttributePermissions,
    pub(crate) control: AttributeControl,
    internal_control: esp_attr_control_t,
    pub(crate) write_callback: Option<fn(Vec<u8>, esp_ble_gatts_cb_param_t_gatts_write_evt_param)>,
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
        callback: fn(Vec<u8>, esp_ble_gatts_cb_param_t_gatts_write_evt_param),
    ) -> &mut Self {
        if !self.permissions.write_access {
            warn!(
                "Descriptor {} does not have write permissions. Ignoring write callback.",
                self
            );

            return self;
        }

        self.write_callback = Some(callback);

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
                leaky_box_raw!(self.uuid.into()),
                self.permissions.into(),
                leaky_box_raw!(esp_attr_value_t {
                    attr_max_len: self.value.len() as u16,
                    attr_len: self.value.len() as u16,
                    attr_value: self.value.as_mut_slice().as_mut_ptr(),
                }),
                &mut self.internal_control,
            ));
        }
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
