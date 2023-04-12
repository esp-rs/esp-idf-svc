use ::log::debug;

use esp_idf_sys::*;

use crate::ble::gatt_server::Profile;
use crate::ble::utilities::AttributeControl;

impl Profile {
    pub(crate) fn on_read(
        &mut self,
        gatts_if: esp_gatt_if_t,
        param: esp_ble_gatts_cb_param_t_gatts_read_evt_param,
    ) {
        for service in &self.services {
            service
                .read()
                .unwrap()
                .characteristics
                .iter()
                .for_each(|characteristic| {
                    if characteristic.read().unwrap().attribute_handle == Some(param.handle) {
                        debug!(
                            "Received read event for characteristic {}.",
                            characteristic.read().unwrap()
                        );

                        // If the characteristic has a read handler, call it.
                        if let AttributeControl::ResponseByApp(callback) =
                            &characteristic.read().unwrap().control
                        {
                            let value = callback(param);

                            // Extend the response to the maximum length.
                            let mut response = [0u8; 600];
                            response[..value.len()].copy_from_slice(&value);

                            let mut esp_rsp = esp_gatt_rsp_t {
                                attr_value: esp_gatt_value_t {
                                    auth_req: 0,
                                    handle: param.handle,
                                    len: value.len() as u16,
                                    offset: 0,
                                    value: response,
                                },
                            };

                            unsafe {
                                esp_nofail!(esp_ble_gatts_send_response(
                                    gatts_if,
                                    param.conn_id,
                                    param.trans_id,
                                    // TODO: Allow different statuses.
                                    esp_gatt_status_t_ESP_GATT_OK,
                                    &mut esp_rsp
                                ));
                            }
                        }
                    } else {
                        characteristic
                            .read()
                            .unwrap()
                            .descriptors
                            .iter()
                            .for_each(|descriptor| {
                                debug!(
                                    "MCC: Checking descriptor {} ({:?}).",
                                    descriptor.read().unwrap(),
                                    descriptor.read().unwrap().attribute_handle
                                );

                                if descriptor.read().unwrap().attribute_handle == Some(param.handle)
                                {
                                    debug!(
                                        "Received read event for descriptor {}.",
                                        descriptor.read().unwrap()
                                    );

                                    if let AttributeControl::ResponseByApp(callback) =
                                        &descriptor.read().unwrap().control
                                    {
                                        let value = callback(param);

                                        // Extend the response to the maximum length.
                                        let mut response = [0u8; 600];
                                        response[..value.len()].copy_from_slice(&value);

                                        let mut esp_rsp = esp_gatt_rsp_t {
                                            attr_value: esp_gatt_value_t {
                                                auth_req: 0,
                                                handle: param.handle,
                                                len: value.len() as u16,
                                                offset: 0,
                                                value: response,
                                            },
                                        };

                                        unsafe {
                                            esp_nofail!(esp_ble_gatts_send_response(
                                                gatts_if,
                                                param.conn_id,
                                                param.trans_id,
                                                esp_gatt_status_t_ESP_GATT_OK,
                                                &mut esp_rsp
                                            ));
                                        }
                                    }
                                }
                            });
                    }
                });
        }
    }
}
