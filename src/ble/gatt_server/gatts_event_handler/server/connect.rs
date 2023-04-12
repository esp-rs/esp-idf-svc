use ::log::info;

use crate::ble::gatt_server::GattServer;
use crate::ble::utilities::Connection;

impl GattServer {
    pub(crate) fn on_connect(
        &mut self,
        param: esp_idf_sys::esp_ble_gatts_cb_param_t_gatts_connect_evt_param,
    ) {
        info!("GATT client {} connected.", Connection::from(param));
        self.active_connections.insert(param.into());
    }

    pub fn is_client_connected(&self) -> bool {
        !self.active_connections.is_empty()
    }
}
