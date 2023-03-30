use crate::ble::gatt_server::{GattServer, Profile};

#[allow(clippy::wildcard_imports)]
use esp_idf_sys::*;
use log::{debug, warn};

mod profile;
mod server;

impl GattServer {
    /// The main GATT server event loop.
    ///
    /// Dispatches the received events across the appropriate profile-related handlers.
    pub(crate) fn gatts_event_handler(
        &mut self,
        event: esp_gatts_cb_event_t,
        gatts_if: esp_gatt_if_t,
        param: *mut esp_ble_gatts_cb_param_t,
    ) {
        #[allow(non_upper_case_globals)]
        match event {
            esp_gatts_cb_event_t_ESP_GATTS_CONNECT_EVT => {
                let param = unsafe { (*param).connect };
                self.on_connect(param);

                // Do not pass this event to the profile handlers.
                return;
            }
            esp_gatts_cb_event_t_ESP_GATTS_DISCONNECT_EVT => {
                let param = unsafe { (*param).disconnect };
                self.on_disconnect(param);

                // Do not pass this event to the profile handlers.
                return;
            }
            esp_gatts_cb_event_t_ESP_GATTS_MTU_EVT => {
                let param = unsafe { (*param).mtu };
                self.on_mtu_change(param);

                // Do not pass this event to the profile handlers.
                return;
            }
            esp_gatts_cb_event_t_ESP_GATTS_REG_EVT => {
                let param = unsafe { (*param).reg };
                self.on_reg(gatts_if, param);

                // Pass this event to the profile handlers.
            }
            esp_gatts_cb_event_t_ESP_GATTS_RESPONSE_EVT => {
                let param = unsafe { (*param).rsp };
                self.on_response(param);

                // Do not pass this event to the profile handlers.
                return;
            }
            esp_gatts_cb_event_t_ESP_GATTS_SET_ATTR_VAL_EVT => {
                let param = unsafe { (*param).set_attr_val };
                self.on_set_attr_val(gatts_if, param);

                // Do not pass this event to the profile handlers.
                return;
            }
            _ => {}
        }

        self.profiles.iter().for_each(|profile| {
            if profile.read().unwrap().interface == Some(gatts_if) {
                debug!(
                    "Handling event {} on profile {}.",
                    event,
                    profile.read().unwrap()
                );
                profile
                    .write()
                    .unwrap()
                    .gatts_event_handler(event, gatts_if, param);
            }
        });
    }
}

impl Profile {
    /// Profile-specific GATT server event loop.
    fn gatts_event_handler(
        &mut self,
        event: esp_gatts_cb_event_t,
        gatts_if: esp_gatt_if_t,
        param: *mut esp_ble_gatts_cb_param_t,
    ) {
        #[allow(non_upper_case_globals)]
        match event {
            esp_gatts_cb_event_t_ESP_GATTS_REG_EVT => {
                let param = unsafe { (*param).reg };

                self.on_reg(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_CREATE_EVT => {
                let param = unsafe { (*param).create };

                self.on_create(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_START_EVT => {
                let param = unsafe { (*param).start };

                self.on_start(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_ADD_CHAR_EVT => {
                let param = unsafe { (*param).add_char };

                self.on_char_add(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_ADD_CHAR_DESCR_EVT => {
                let param = unsafe { (*param).add_char_descr };

                self.on_char_add_descr(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_WRITE_EVT => {
                let param = unsafe { (*param).write };

                self.on_write(gatts_if, param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_READ_EVT => {
                let param = unsafe { (*param).read };

                self.on_read(gatts_if, param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_CONF_EVT => {
                let _param = unsafe { (*param).conf };

                // TODO: on_conf.
                debug!("Received confirmation event.");
            }
            _ => {
                warn!("Unhandled GATT server event: {:?}", event);
            }
        }
    }
}
