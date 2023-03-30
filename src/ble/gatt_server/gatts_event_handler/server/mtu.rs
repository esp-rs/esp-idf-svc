use crate::ble::gatt_server::GattServer;
use log::debug;

impl GattServer {
    #[allow(clippy::unused_self)]
    pub(crate) fn on_mtu_change(
        &self,
        param: esp_idf_sys::esp_ble_gatts_cb_param_t_gatts_mtu_evt_param,
    ) {
        debug!("MTU changed to {}.", param.mtu);
    }
}
