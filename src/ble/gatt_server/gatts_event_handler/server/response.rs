use crate::gatt_server::GattServer;
use log::debug;

impl GattServer {
    #[allow(clippy::unused_self)]
    pub(crate) fn on_response(
        &self,
        param: esp_idf_sys::esp_ble_gatts_cb_param_t_gatts_rsp_evt_param,
    ) {
        debug!("Responded to handle 0x{:04x}.", param.handle);
    }
}
