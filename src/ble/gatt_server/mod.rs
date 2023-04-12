//! The GATT server.
#![allow(clippy::cast_possible_truncation)]
// Structs.
mod characteristic;
mod descriptor;
mod profile;
mod service;

// Custom stuff.
mod custom_attributes;

// Event handler.
mod gap_event_handler;
mod gatts_event_handler;

use alloc::{boxed::Box, sync::Arc};
use std::{collections::HashSet, sync::RwLock};

use ::log::{info, warn};

use esp_idf_sys::*;

use crate::{
    ble::utilities::{Appearance, Connection},
    private::mutex::{Mutex, RawMutex},
};

pub use characteristic::Characteristic;
pub use descriptor::Descriptor;
pub use profile::Profile;
pub use service::Service;

type Singleton<T> = Mutex<Option<Box<T>>>;

/// The GATT server singleton.
pub static GLOBAL_GATT_SERVER: Singleton<GattServer> = Mutex::wrap(RawMutex::new(), None);

/// Represents a GATT server.
///
/// This is a singleton, and can be accessed via the [`GLOBAL_GATT_SERVER`] static.
pub struct GattServer {
    profiles: Vec<Arc<RwLock<Profile>>>,
    started: bool,
    advertisement_parameters: esp_ble_adv_params_t,
    advertisement_data: esp_ble_adv_data_t,
    scan_response_data: esp_ble_adv_data_t,
    device_name: String,
    advertisement_configured: bool,
    active_connections: HashSet<Connection>,
}

unsafe impl Send for GattServer {}

/// The GATT server singleton initialization.
///
/// Must be called before the GATT server usage.
pub fn init() {
    *GLOBAL_GATT_SERVER.lock() = Some(Box::new(GattServer {
        profiles: Vec::new(),
        started: false,
        advertisement_parameters: esp_ble_adv_params_t {
            adv_int_min: 0x20,
            adv_int_max: 0x40,
            adv_type: esp_ble_adv_type_t_ADV_TYPE_IND,
            own_addr_type: esp_ble_addr_type_t_BLE_ADDR_TYPE_RPA_PUBLIC,
            channel_map: esp_ble_adv_channel_t_ADV_CHNL_ALL,
            adv_filter_policy: esp_ble_adv_filter_t_ADV_FILTER_ALLOW_SCAN_ANY_CON_ANY,
            ..Default::default()
        },
        advertisement_data: esp_ble_adv_data_t {
            set_scan_rsp: false,
            include_name: true,
            include_txpower: true,
            min_interval: 0x0006,
            max_interval: 0x0010,
            appearance: Appearance::GenericUnknown.into(),
            manufacturer_len: 0,
            p_manufacturer_data: std::ptr::null_mut(),
            service_data_len: 0,
            p_service_data: std::ptr::null_mut(),
            service_uuid_len: 0,
            p_service_uuid: std::ptr::null_mut(),
            flag: (ESP_BLE_ADV_FLAG_GEN_DISC | ESP_BLE_ADV_FLAG_BREDR_NOT_SPT) as u8,
        },
        scan_response_data: esp_ble_adv_data_t {
            set_scan_rsp: true,
            include_name: false,
            include_txpower: false,
            min_interval: 0x0006,
            max_interval: 0x0010,
            appearance: Appearance::GenericUnknown.into(),
            manufacturer_len: 0,
            p_manufacturer_data: std::ptr::null_mut(),
            service_data_len: 0,
            p_service_data: std::ptr::null_mut(),
            service_uuid_len: 0,
            p_service_uuid: std::ptr::null_mut(),
            flag: (ESP_BLE_ADV_FLAG_GEN_DISC | ESP_BLE_ADV_FLAG_BREDR_NOT_SPT) as u8,
        },
        advertisement_configured: false,
        device_name: "ESP32".to_string(),
        active_connections: HashSet::new(),
    }));
}

impl GattServer {
    /// Starts a [`GattServer`].
    ///
    /// # Panics
    ///
    /// Panics if a profile's lock is poisoned.
    pub fn start(&mut self) -> &mut Self {
        if self.started {
            warn!("GATT server already started.");
            return self;
        }

        self.started = true;
        Self::initialise_ble_stack();

        // Registration of profiles, services, characteristics and descriptors.
        self.profiles.iter().for_each(|profile| {
            profile.write().unwrap().register_self();
        });

        self
    }

    /// Sets the name to be advertised in GAP packets.
    ///
    /// The name must be set before starting the GATT server.
    pub fn device_name<S: Into<String>>(&mut self, name: S) -> &mut Self {
        if self.advertisement_configured {
            warn!(
                "Device name already set. Please set the device name before starting the server."
            );
            return self;
        }

        self.device_name = name.into();
        self.device_name.push('\0');

        self
    }

    /// Sets the device appearance value to be advertised in GAP packets.
    pub fn appearance(&mut self, appearance: Appearance) -> &mut Self {
        if self.advertisement_configured {
            warn!("Appearance already set. Please set the appearance before starting the server.");
            return self;
        }

        self.advertisement_data.appearance = appearance.into();
        self.scan_response_data.appearance = appearance.into();

        self
    }

    /// Sets the raw GAP advertisement parameters.
    pub fn set_adv_params(&mut self, params: esp_ble_adv_params_t) -> &mut Self {
        self.advertisement_parameters = params;
        self
    }

    /// Sets the raw GAP advertisement data.
    pub fn set_adv_data(&mut self, data: esp_ble_adv_data_t) -> &mut Self {
        self.advertisement_data = data;

        self
    }

    /// Advertises the specified [`Service`] in GAP packets.
    ///
    /// # Panics
    ///
    /// Panics if the service lock is poisoned.
    pub fn advertise_service(&mut self, service: &Arc<RwLock<Service>>) -> &mut Self {
        let mut uuid = service.read().unwrap().uuid.as_uuid128_array();
        self.scan_response_data.p_service_uuid = uuid.as_mut_ptr();
        self.scan_response_data.service_uuid_len = uuid.len() as u16;

        self
    }

    /// Add a [`Profile`] to the GATT server.
    pub fn profile(&mut self, profile: Arc<RwLock<Profile>>) -> &mut Self {
        if self.started {
            warn!("Cannot add profile after server has started.");
            return self;
        }

        self.profiles.push(profile);
        self
    }

    /// Setup GattServer mtu that will be used to negotiate mtu during request from client peer
    /// # Arguments
    /// * `mtu` -  value to set local mtu, should be larger than 23 and lower or equal to 517
    #[allow(unused)]
    pub fn set_mtu(&mut self, mtu: u16) -> &mut Self {
        unsafe {
            esp_idf_sys::esp_nofail!(esp_idf_sys::esp_ble_gatt_set_local_mtu(mtu));
        }

        self
    }

    pub(crate) fn get_profile(&self, interface: u8) -> Option<Arc<RwLock<Profile>>> {
        self.profiles
            .iter()
            .find(|profile| profile.write().unwrap().interface == Some(interface))
            .cloned()
    }

    #[allow(clippy::too_many_lines)]
    fn initialise_ble_stack() {
        info!("Initialising BLE stack.");

        #[cfg(esp32)]
        let mut default_controller_configuration = esp_bt_controller_config_t {
            controller_task_stack_size: ESP_TASK_BT_CONTROLLER_STACK as _,
            controller_task_prio: ESP_TASK_BT_CONTROLLER_PRIO as _,
            hci_uart_no: BT_HCI_UART_NO_DEFAULT as _,
            hci_uart_baudrate: BT_HCI_UART_BAUDRATE_DEFAULT,
            scan_duplicate_mode: SCAN_DUPLICATE_MODE as _,
            scan_duplicate_type: SCAN_DUPLICATE_TYPE_VALUE as _,
            normal_adv_size: NORMAL_SCAN_DUPLICATE_CACHE_SIZE as _,
            mesh_adv_size: MESH_DUPLICATE_SCAN_CACHE_SIZE as _,
            send_adv_reserved_size: SCAN_SEND_ADV_RESERVED_SIZE as _,
            controller_debug_flag: CONTROLLER_ADV_LOST_DEBUG_BIT,
            mode: esp_bt_mode_t_ESP_BT_MODE_BLE as _,
            ble_max_conn: CONFIG_BTDM_CTRL_BLE_MAX_CONN_EFF as _,
            bt_max_acl_conn: CONFIG_BTDM_CTRL_BR_EDR_MAX_ACL_CONN_EFF as _,
            bt_sco_datapath: CONFIG_BTDM_CTRL_BR_EDR_SCO_DATA_PATH_EFF as _,
            auto_latency: BTDM_CTRL_AUTO_LATENCY_EFF != 0,
            bt_legacy_auth_vs_evt: BTDM_CTRL_LEGACY_AUTH_VENDOR_EVT_EFF != 0,
            bt_max_sync_conn: CONFIG_BTDM_CTRL_BR_EDR_MAX_SYNC_CONN_EFF as _,
            ble_sca: CONFIG_BTDM_BLE_SLEEP_CLOCK_ACCURACY_INDEX_EFF as _,
            pcm_role: CONFIG_BTDM_CTRL_PCM_ROLE_EFF as _,
            pcm_polar: CONFIG_BTDM_CTRL_PCM_POLAR_EFF as _,
            hli: BTDM_CTRL_HLI != 0,
            magic: ESP_BT_CONTROLLER_CONFIG_MAGIC_VAL,
            dup_list_refresh_period: SCAN_DUPL_CACHE_REFRESH_PERIOD as u16,
        };

        #[cfg(esp32c3)]
        let mut default_controller_configuration = esp_bt_controller_config_t {
            magic: ESP_BT_CTRL_CONFIG_MAGIC_VAL,
            version: ESP_BT_CTRL_CONFIG_VERSION,
            controller_task_stack_size: ESP_TASK_BT_CONTROLLER_STACK as u16,
            controller_task_prio: ESP_TASK_BT_CONTROLLER_PRIO as u8,
            controller_task_run_cpu: CONFIG_BT_CTRL_PINNED_TO_CORE as u8,
            bluetooth_mode: CONFIG_BT_CTRL_MODE_EFF as u8,
            ble_max_act: CONFIG_BT_CTRL_BLE_MAX_ACT_EFF as u8,
            sleep_mode: CONFIG_BT_CTRL_SLEEP_MODE_EFF as u8,
            sleep_clock: CONFIG_BT_CTRL_SLEEP_CLOCK_EFF as u8,
            ble_st_acl_tx_buf_nb: CONFIG_BT_CTRL_BLE_STATIC_ACL_TX_BUF_NB as u8,
            ble_hw_cca_check: CONFIG_BT_CTRL_HW_CCA_EFF as u8,
            ble_adv_dup_filt_max: CONFIG_BT_CTRL_ADV_DUP_FILT_MAX as u16,
            coex_param_en: false,
            ce_len_type: CONFIG_BT_CTRL_CE_LENGTH_TYPE_EFF as u8,
            coex_use_hooks: false,
            hci_tl_type: CONFIG_BT_CTRL_HCI_TL_EFF as u8,
            hci_tl_funcs: std::ptr::null_mut(),
            txant_dft: CONFIG_BT_CTRL_TX_ANTENNA_INDEX_EFF as u8,
            rxant_dft: CONFIG_BT_CTRL_RX_ANTENNA_INDEX_EFF as u8,
            txpwr_dft: CONFIG_BT_CTRL_DFT_TX_POWER_LEVEL_EFF as u8,
            cfg_mask: CFG_NASK,
            scan_duplicate_mode: SCAN_DUPLICATE_MODE as u8,
            scan_duplicate_type: SCAN_DUPLICATE_TYPE_VALUE as u8,
            normal_adv_size: NORMAL_SCAN_DUPLICATE_CACHE_SIZE as u16,
            mesh_adv_size: MESH_DUPLICATE_SCAN_CACHE_SIZE as u16,
            coex_phy_coded_tx_rx_time_limit: CONFIG_BT_CTRL_COEX_PHY_CODED_TX_RX_TLIM_EFF as u8,
            #[cfg(any(esp_idf_version = "4.4", esp_idf_version = "5.0"))]
            hw_target_code: BLE_HW_TARGET_CODE_ESP32C3_CHIP_ECO0,
            #[cfg(esp_idf_version = "5.1")]
            hw_target_code: BLE_HW_TARGET_CODE_CHIP_ECO0,
            slave_ce_len_min: SLAVE_CE_LEN_MIN_DEFAULT as u8,
            hw_recorrect_en: AGC_RECORRECT_EN as u8,
            cca_thresh: CONFIG_BT_CTRL_HW_CCA_VAL as u8,
            #[cfg(any(esp_idf_version = "5.0", esp_idf_version = "5.1"))]
            scan_backoff_upperlimitmax: CONFIG_BT_CTRL_SCAN_BACKOFF_UPPERLIMITMAX as u16,
            #[cfg(any(esp_idf_version = "5.0", esp_idf_version = "5.1"))]
            dup_list_refresh_period: CONFIG_DUPL_SCAN_CACHE_REFRESH_PERIOD as u16,
            #[cfg(esp_idf_version = "5.1")]
            ble_50_feat_supp: BT_CTRL_50_FEATURE_SUPPORT != 0,
        };

        #[cfg(esp32s3)]
        let mut default_controller_configuration = esp_bt_controller_config_t {
            magic: ESP_BT_CTRL_CONFIG_MAGIC_VAL,
            version: ESP_BT_CTRL_CONFIG_VERSION,
            controller_task_stack_size: ESP_TASK_BT_CONTROLLER_STACK as u16,
            controller_task_prio: ESP_TASK_BT_CONTROLLER_PRIO as u8,
            controller_task_run_cpu: CONFIG_BT_CTRL_PINNED_TO_CORE as u8,
            bluetooth_mode: CONFIG_BT_CTRL_MODE_EFF as u8,
            ble_max_act: CONFIG_BT_CTRL_BLE_MAX_ACT_EFF as u8,
            sleep_mode: CONFIG_BT_CTRL_SLEEP_MODE_EFF as u8,
            sleep_clock: CONFIG_BT_CTRL_SLEEP_CLOCK_EFF as u8,
            ble_st_acl_tx_buf_nb: CONFIG_BT_CTRL_BLE_STATIC_ACL_TX_BUF_NB as u8,
            ble_hw_cca_check: CONFIG_BT_CTRL_HW_CCA_EFF as u8,
            ble_adv_dup_filt_max: CONFIG_BT_CTRL_ADV_DUP_FILT_MAX as u16,
            coex_param_en: false,
            ce_len_type: CONFIG_BT_CTRL_CE_LENGTH_TYPE_EFF as u8,
            coex_use_hooks: false,
            hci_tl_type: CONFIG_BT_CTRL_HCI_TL_EFF as u8,
            hci_tl_funcs: std::ptr::null_mut(),
            txant_dft: CONFIG_BT_CTRL_TX_ANTENNA_INDEX_EFF as u8,
            rxant_dft: CONFIG_BT_CTRL_RX_ANTENNA_INDEX_EFF as u8,
            txpwr_dft: CONFIG_BT_CTRL_DFT_TX_POWER_LEVEL_EFF as u8,
            cfg_mask: CFG_MASK,
            scan_duplicate_mode: SCAN_DUPLICATE_MODE as u8,
            scan_duplicate_type: SCAN_DUPLICATE_TYPE_VALUE as u8,
            normal_adv_size: NORMAL_SCAN_DUPLICATE_CACHE_SIZE as u16,
            mesh_adv_size: MESH_DUPLICATE_SCAN_CACHE_SIZE as u16,
            coex_phy_coded_tx_rx_time_limit: CONFIG_BT_CTRL_COEX_PHY_CODED_TX_RX_TLIM_EFF as u8,
            #[cfg(any(esp_idf_version = "4.4", esp_idf_version = "5.0"))]
            hw_target_code: BLE_HW_TARGET_CODE_ESP32S3_CHIP_ECO0,
            #[cfg(esp_idf_version = "5.1")]
            hw_target_code: BLE_HW_TARGET_CODE_CHIP_ECO0,
            slave_ce_len_min: SLAVE_CE_LEN_MIN_DEFAULT as u8,
            hw_recorrect_en: AGC_RECORRECT_EN as u8,
            cca_thresh: CONFIG_BT_CTRL_HW_CCA_VAL as u8,
            #[cfg(any(esp_idf_version = "5.0", esp_idf_version = "5.1"))]
            scan_backoff_upperlimitmax: CONFIG_BT_CTRL_SCAN_BACKOFF_UPPERLIMITMAX as u16,
            #[cfg(any(esp_idf_version = "5.0", esp_idf_version = "5.1"))]
            dup_list_refresh_period: CONFIG_DUPL_SCAN_CACHE_REFRESH_PERIOD as u16,
            #[cfg(esp_idf_version = "5.1")]
            ble_50_feat_supp: BT_CTRL_50_FEATURE_SUPPORT != 0,
        };
        // BLE controller initialisation.
        unsafe {
            esp_nofail!(esp_bt_controller_mem_release(
                esp_bt_mode_t_ESP_BT_MODE_CLASSIC_BT
            ));
            esp_nofail!(esp_bt_controller_init(
                &mut default_controller_configuration
            ));
            esp_nofail!(esp_bt_controller_enable(esp_bt_mode_t_ESP_BT_MODE_BLE));
            esp_nofail!(esp_bluedroid_init());
            esp_nofail!(esp_bluedroid_enable());
            esp_nofail!(esp_ble_gatts_register_callback(Some(
                Self::default_gatts_callback
            )));
            esp_nofail!(esp_ble_gap_register_callback(Some(
                Self::default_gap_callback
            )));
        }
    }

    /// Calls the global server's GATT event callback.
    ///
    /// This is a bad workaround, and only works because we have a singleton server.
    extern "C" fn default_gatts_callback(
        event: esp_gatts_cb_event_t,
        gatts_if: esp_gatt_if_t,
        param: *mut esp_ble_gatts_cb_param_t,
    ) {
        match &mut *GLOBAL_GATT_SERVER.lock() {
            Some(gatt_server) => gatt_server.gatts_event_handler(event, gatts_if, param),
            None => panic!("GATT server not initialized"),
        }
    }

    /// Calls the global server's GAP event callback.
    ///
    /// This is a bad workaround, and only works because we have a singleton server.
    extern "C" fn default_gap_callback(
        event: esp_gap_ble_cb_event_t,
        param: *mut esp_ble_gap_cb_param_t,
    ) {
        match &mut *GLOBAL_GATT_SERVER.lock() {
            Some(gatt_server) => gatt_server.gap_event_handler(event, param),
            None => panic!("GATT server not initialized"),
        }
    }
}
