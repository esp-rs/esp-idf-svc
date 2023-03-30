use crate::ble::gatt_server::Profile;
use esp_idf_sys::*;
use log::{debug, warn};

impl Profile {
    pub(crate) fn on_start(&mut self, param: esp_ble_gatts_cb_param_t_gatts_start_evt_param) {
        let Some(service) = self.get_service(param.service_handle) else {
            warn!("Cannot find service described by service handle {} received in start event.", param.service_handle);
            return;
        };

        if param.status == esp_gatt_status_t_ESP_GATT_OK {
            debug!("GATT service {} started.", service.read().unwrap());
        } else {
            warn!("GATT service {} failed to start.", service.read().unwrap());
        }
    }
}
