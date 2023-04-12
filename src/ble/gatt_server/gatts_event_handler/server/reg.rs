use ::log::debug;

use esp_idf_sys::*;

use crate::ble::gatt_server::GattServer;

impl GattServer {
    pub(crate) fn on_reg(
        &mut self,
        gatts_if: esp_gatt_if_t,
        param: esp_ble_gatts_cb_param_t_gatts_reg_evt_param,
    ) {
        if param.status == esp_gatt_status_t_ESP_GATT_OK {
            debug!("New profile registered.");

            let profile = self
                .profiles
                .iter()
                .find(|profile| (*profile).read().unwrap().identifier == param.app_id)
                .expect("No profile found with received application identifier.");

            profile.write().unwrap().interface = Some(gatts_if);

            if !self.advertisement_configured {
                unsafe {
                    esp_nofail!(esp_ble_gap_set_device_name(
                        self.device_name.as_ptr().cast::<i8>()
                    ));

                    self.advertisement_configured = true;

                    // Advertisement data.
                    esp_nofail!(esp_ble_gap_config_adv_data(&mut self.advertisement_data));

                    // Scan response data.
                    esp_nofail!(esp_ble_gap_config_adv_data(&mut self.scan_response_data));
                }
            }
        }
    }
}
