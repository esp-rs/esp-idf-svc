use ::log::{info, warn};

use esp_idf_sys::*;

use crate::ble::gatt_server::Profile;

impl Profile {
    pub(crate) fn on_reg(&mut self, param: esp_ble_gatts_cb_param_t_gatts_reg_evt_param) {
        // Check status
        if param.status == esp_bt_status_t_ESP_BT_STATUS_SUCCESS {
            info!(
                "{} registered on interface {}.",
                &self,
                self.interface.unwrap()
            );
            self.register_services();
        } else {
            warn!("GATT profile registration failed.");
        }
    }
}
