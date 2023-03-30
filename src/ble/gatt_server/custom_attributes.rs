use std::sync::{Arc, Mutex};

use crate::{
    ble::gatt_server::Descriptor,
    ble::utilities::{AttributePermissions, BleUuid},
    nvs::EspDefaultNvs,
};

use log::debug;

impl Descriptor {
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
                            .set_cccd_value(key, value.clone());
                    }
                }
            })
            .clone()
    }
}
