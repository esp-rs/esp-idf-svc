use esp_idf_sys::{
    esp_ble_gap_cb_param_t, esp_ble_gap_start_advertising, esp_bt_status_t_ESP_BT_STATUS_SUCCESS,
    esp_gap_ble_cb_event_t, esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_DATA_SET_COMPLETE_EVT,
    esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_START_COMPLETE_EVT,
    esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_STOP_COMPLETE_EVT,
    esp_gap_ble_cb_event_t_ESP_GAP_BLE_SCAN_RSP_DATA_SET_COMPLETE_EVT,
    esp_gap_ble_cb_event_t_ESP_GAP_BLE_UPDATE_CONN_PARAMS_EVT, esp_nofail,
};

use log::{debug, info, warn};

use super::GattServer;
use crate::leaky_box_raw;

impl GattServer {
    pub(crate) extern "C" fn gap_event_handler(
        &mut self,
        event: esp_gap_ble_cb_event_t,
        param: *mut esp_ble_gap_cb_param_t,
    ) {
        #[allow(non_upper_case_globals)]
        match event {
            esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_DATA_SET_COMPLETE_EVT => {
                debug!("BLE GAP advertisement data set complete.");
                info!("Starting BLE GAP advertisement.");

                unsafe {
                    esp_nofail!(esp_ble_gap_start_advertising(leaky_box_raw!(
                        self.advertisement_parameters
                    )));
                }
            }
            esp_gap_ble_cb_event_t_ESP_GAP_BLE_SCAN_RSP_DATA_SET_COMPLETE_EVT => {
                debug!("BLE GAP scan response data set complete.");
                info!("Starting BLE GAP response advertisement.");

                unsafe {
                    esp_nofail!(esp_ble_gap_start_advertising(leaky_box_raw!(
                        self.advertisement_parameters
                    )));
                }
            }
            esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_START_COMPLETE_EVT => {
                let param = unsafe { (*param).adv_data_cmpl };
                if param.status == esp_bt_status_t_ESP_BT_STATUS_SUCCESS {
                    debug!("BLE GAP advertisement started.");
                } else {
                    warn!("BLE GAP advertisement start failed.");
                }
            }
            esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_STOP_COMPLETE_EVT => {
                let param = unsafe { (*param).adv_data_cmpl };
                if param.status == esp_bt_status_t_ESP_BT_STATUS_SUCCESS {
                    debug!("BLE GAP advertisement stopped.");
                } else {
                    warn!("BLE GAP advertisement stop failed.");
                }
            }
            esp_gap_ble_cb_event_t_ESP_GAP_BLE_UPDATE_CONN_PARAMS_EVT => {
                let param = unsafe { (*param).update_conn_params };
                info!("Connection parameters updated: {:?}", param);
            }
            _ => {
                warn!("Unhandled GAP event: {:?}", event);
            }
        }
    }
}
