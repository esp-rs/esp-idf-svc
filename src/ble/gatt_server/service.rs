use std::sync::{Arc, RwLock};

use ::log::debug;

use esp_idf_sys::*;

use crate::{
    ble::gatt_server::characteristic::Characteristic, ble::gatt_server::descriptor::Descriptor,
    ble::utilities::BleUuid,
};

/// Represents a GATT service.
#[derive(Debug, Clone)]
pub struct Service {
    name: Option<String>,
    pub(crate) uuid: BleUuid,
    pub(crate) characteristics: Vec<Arc<RwLock<Characteristic>>>,
    primary: bool,
    pub(crate) handle: Option<u16>,
}

impl Service {
    /// Creates a new [`Service`].
    #[must_use]
    pub const fn new(uuid: BleUuid) -> Self {
        Self {
            name: None,
            uuid,
            characteristics: Vec::new(),
            primary: false,
            handle: None,
        }
    }

    /// Sets the name of the [`Service`].
    ///
    /// This name is only used for debugging purposes.
    pub fn name<S: Into<String>>(&mut self, name: S) -> &mut Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the [`Service`] as primary.
    ///
    /// If you want your service to show up after an interrogation, you need to set it as primary.
    pub fn primary(&mut self) -> &mut Self {
        self.primary = true;
        self
    }

    /// Adds a [`Characteristic`] to the [`Service`].
    pub fn characteristic(&mut self, characteristic: &Arc<RwLock<Characteristic>>) -> &mut Self {
        self.characteristics.push(characteristic.clone());
        self
    }

    /// Returns a reference to the built [`Service`] behind an `Arc` and an `RwLock`.
    ///
    /// The returned value can be passed to any function of this crate that expects a [`Service`].
    /// It can be used in different threads, because it is protected by an `RwLock`.
    #[must_use]
    pub fn build(&self) -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(self.clone()))
    }

    pub(crate) fn get_characteristic_by_handle(
        &self,
        handle: u16,
    ) -> Option<Arc<RwLock<Characteristic>>> {
        self.characteristics
            .iter()
            .find(|characteristic| characteristic.read().unwrap().attribute_handle == Some(handle))
            .cloned()
    }

    pub(crate) fn get_characteristic_by_id(
        &self,
        id: esp_bt_uuid_t,
    ) -> Option<Arc<RwLock<Characteristic>>> {
        self.characteristics
            .iter()
            .find(|characteristic| characteristic.read().unwrap().uuid == id.into())
            .cloned()
    }

    pub(crate) fn get_descriptors_by_id(&self, id: esp_bt_uuid_t) -> Vec<Arc<RwLock<Descriptor>>> {
        self.characteristics
            .iter()
            .filter_map(|characteristic| {
                characteristic
                    .read()
                    .unwrap()
                    .clone()
                    .descriptors
                    .into_iter()
                    .find(|descriptor| descriptor.read().unwrap().uuid == id.into())
            })
            .collect()
    }

    pub(crate) fn register_self(&mut self, interface: u8) {
        debug!("Registering {} on interface {}.", &self, interface);

        let mut id: esp_gatt_srvc_id_t = esp_gatt_srvc_id_t {
            id: self.uuid.into(),
            is_primary: self.primary,
        };

        unsafe {
            esp_nofail!(esp_ble_gatts_create_service(
                interface, &mut id,
                256, // TODO: count the number of characteristics and descriptors.
            ));
        }
    }

    pub(crate) fn register_characteristics(&mut self) {
        debug!("Registering {}'s characteristics.", &self);

        // Attention: The characteristics should be registered one after another.
        // We need to wait for the previous characteristic to be registered before we can register the next one.

        if self.characteristics.is_empty() {
            return;
        }

        let service_handle = self.handle.unwrap();
        let characteristics = self.characteristics.clone();
        std::thread::spawn(move || {
            for c in characteristics {
                c.write().unwrap().register_self(service_handle);
                while c.read().unwrap().attribute_handle.is_none() {
                    std::thread::yield_now();
                }
            }
        });
    }
}

impl core::fmt::Display for Service {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} ({})",
            self.name
                .clone()
                .unwrap_or_else(|| "Unnamed service".to_string()),
            self.uuid,
        )
    }
}
