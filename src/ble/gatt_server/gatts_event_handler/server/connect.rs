use crate::gatt_server::GattServer;
use crate::utilities::Connection;
use log::info;

impl GattServer {
    pub(crate) fn on_connect(
        &mut self,
        param: esp_idf_sys::esp_ble_gatts_cb_param_t_gatts_connect_evt_param,
    ) {
        info!("GATT client {} connected.", Connection::from(param));
        self.active_connections.insert(param.into());
    }
}
