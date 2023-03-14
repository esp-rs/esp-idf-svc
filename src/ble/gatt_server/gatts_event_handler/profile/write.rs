use crate::ble::gatt_server::Profile;
use crate::ble::utilities::AttributeControl;
use esp_idf_sys::*;
use log::debug;

impl Profile {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn on_write(
        &mut self,
        gatts_if: esp_gatt_if_t,
        param: esp_ble_gatts_cb_param_t_gatts_write_evt_param,
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
                            "Received write event for characteristic {}.",
                            characteristic.read().unwrap()
                        );

                        // If the characteristic has a write handler, call it.
                        if let Some(write_callback) = &characteristic.read().unwrap().write_callback
                        {
                            let value = unsafe {
                                std::slice::from_raw_parts(param.value, param.len as usize)
                            }
                            .to_vec();

                            write_callback(value, param);

                            // Send response if needed.
                            if param.need_rsp {
                                if let AttributeControl::ResponseByApp(read_callback) =
                                    &characteristic.read().unwrap().control
                                {
                                    // Simulate a read operation.
                                    let param_as_read_operation =
                                        esp_ble_gatts_cb_param_t_gatts_read_evt_param {
                                            bda: param.bda,
                                            conn_id: param.conn_id,
                                            handle: param.handle,
                                            need_rsp: param.need_rsp,
                                            offset: param.offset,
                                            trans_id: param.trans_id,
                                            ..Default::default()
                                        };

                                    // Get value.
                                    let value = read_callback(param_as_read_operation);

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
                        }
                    } else {
                        characteristic
                            .read()
                            .unwrap()
                            .descriptors
                            .iter()
                            .for_each(|descriptor| {
                                if descriptor.read().unwrap().attribute_handle == Some(param.handle)
                                {
                                    debug!(
                                        "Received write event for descriptor {}.",
                                        descriptor.read().unwrap()
                                    );

                                    if let Some(write_callback) =
                                        descriptor.read().unwrap().write_callback
                                    {
                                        let value = unsafe {
                                            std::slice::from_raw_parts(
                                                param.value,
                                                param.len as usize,
                                            )
                                        }
                                        .to_vec();

                                        write_callback(value, param);

                                        // Send response if needed.
                                        if param.need_rsp {
                                            if let AttributeControl::ResponseByApp(read_callback) =
                                                &descriptor.read().unwrap().control
                                            {
                                                // Simulate a read operation.
                                                let param_as_read_operation =
                                                    esp_ble_gatts_cb_param_t_gatts_read_evt_param {
                                                        bda: param.bda,
                                                        conn_id: param.conn_id,
                                                        handle: param.handle,
                                                        need_rsp: param.need_rsp,
                                                        offset: param.offset,
                                                        trans_id: param.trans_id,
                                                        ..Default::default()
                                                    };

                                                // Get value.
                                                let value = read_callback(param_as_read_operation);

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
                                    }
                                }
                            });
                    }
                });
        }
    }
}
