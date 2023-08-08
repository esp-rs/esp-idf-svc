use core::marker::PhantomData;

use esp_idf_hal::modem::BluetoothModemPeripheral;
use esp_idf_hal::peripheral::Peripheral;

use esp_idf_sys::*;
use log::info;

#[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
use crate::nvs::EspDefaultNvsPartition;

pub mod ble;

pub trait BtMode {
    fn mode() -> esp_bt_mode_t;
}

pub trait BleEnabled {}

pub trait BtClassicEnabled {}

pub struct BtClassic;

impl BtClassicEnabled for BtClassic {}

impl BtMode for BtClassic {
    fn mode() -> esp_bt_mode_t {
        esp_bt_mode_t_ESP_BT_MODE_BLE
    }
}

pub struct Ble;

impl BleEnabled for Ble {}

impl BtMode for Ble {
    fn mode() -> esp_bt_mode_t {
        esp_bt_mode_t_ESP_BT_MODE_CLASSIC_BT
    }
}

pub struct BtDual;

impl BtClassicEnabled for BtDual {}
impl BleEnabled for BtDual {}

impl BtMode for BtDual {
    fn mode() -> esp_bt_mode_t {
        esp_bt_mode_t_ESP_BT_MODE_BTDM
    }
}

#[derive(Debug, Clone)]
pub enum BtUuid {
    Uuid16([u8; 2]),
    Uuid32([u8; 4]),
    Uuid128([u8; 16]),
}

impl BtUuid {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            BtUuid::Uuid16(uuid) => uuid,
            BtUuid::Uuid32(uuid) => uuid,
            BtUuid::Uuid128(uuid) => uuid,
        }
    }
}

impl From<&BtUuid> for esp_bt_uuid_t {
    fn from(uuid: &BtUuid) -> Self {
        let mut bt_uuid: esp_bt_uuid_t = Default::default();

        match uuid {
            BtUuid::Uuid16(uuid) => bt_uuid.uuid.uuid16 = u16::from_le_bytes(uuid.clone()),
            BtUuid::Uuid32(uuid) => bt_uuid.uuid.uuid32 = u32::from_le_bytes(uuid.clone()),
            BtUuid::Uuid128(uuid) => bt_uuid.uuid.uuid128 = uuid.clone(),
        }

        bt_uuid.len = uuid.as_bytes().len() as _;

        bt_uuid
    }
}

pub struct BtDriver<'d, M>
where
    M: BtMode,
{
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    _nvs: Option<EspDefaultNvsPartition>,
    _p: PhantomData<&'d mut ()>,
    _m: PhantomData<M>,
}

impl<'d, M> BtDriver<'d, M>
where
    M: BtMode,
{
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    pub fn new<B: BluetoothModemPeripheral>(
        _modem: impl Peripheral<P = B> + 'd,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> Result<Self, EspError> {
        Self::init(nvs.is_some())?;

        Ok(Self {
            _nvs: nvs,
            _p: PhantomData,
            _m: PhantomData,
        })
    }

    #[cfg(not(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled)))]
    pub fn new<B: BluetoothModemPeripheral>(
        _modem: impl Peripheral<P = B> + 'd,
    ) -> Result<Self, EspError> {
        Self::init(false)?;

        Ok(Self {
            _p: PhantomData,
            _m: PhantomData,
        })
    }

    fn init(_nvs_enabled: bool) -> Result<(), EspError> {
        #[cfg(esp32)]
        let mut bt_cfg = esp_bt_controller_config_t {
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
            mode: M::mode() as _,
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
            dup_list_refresh_period: SCAN_DUPL_CACHE_REFRESH_PERIOD as _,
        };

        #[cfg(esp32c3)]
        let mut bt_cfg = esp_bt_controller_config_t {
            magic: esp_idf_sys::ESP_BT_CTRL_CONFIG_MAGIC_VAL,
            version: esp_idf_sys::ESP_BT_CTRL_CONFIG_VERSION,
            controller_task_stack_size: esp_idf_sys::ESP_TASK_BT_CONTROLLER_STACK as _,
            controller_task_prio: esp_idf_sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
            controller_task_run_cpu: esp_idf_sys::CONFIG_BT_CTRL_PINNED_TO_CORE as _,
            bluetooth_mode: esp_idf_sys::CONFIG_BT_CTRL_MODE_EFF as _,
            ble_max_act: esp_idf_sys::CONFIG_BT_CTRL_BLE_MAX_ACT_EFF as _,
            sleep_mode: esp_idf_sys::CONFIG_BT_CTRL_SLEEP_MODE_EFF as _,
            sleep_clock: esp_idf_sys::CONFIG_BT_CTRL_SLEEP_CLOCK_EFF as _,
            ble_st_acl_tx_buf_nb: esp_idf_sys::CONFIG_BT_CTRL_BLE_STATIC_ACL_TX_BUF_NB as _,
            ble_hw_cca_check: esp_idf_sys::CONFIG_BT_CTRL_HW_CCA_EFF as _,
            ble_adv_dup_filt_max: esp_idf_sys::CONFIG_BT_CTRL_ADV_DUP_FILT_MAX as _,
            ce_len_type: esp_idf_sys::CONFIG_BT_CTRL_CE_LENGTH_TYPE_EFF as _,
            hci_tl_type: esp_idf_sys::CONFIG_BT_CTRL_HCI_TL_EFF as _,
            hci_tl_funcs: std::ptr::null_mut(),
            txant_dft: esp_idf_sys::CONFIG_BT_CTRL_TX_ANTENNA_INDEX_EFF as _,
            rxant_dft: esp_idf_sys::CONFIG_BT_CTRL_RX_ANTENNA_INDEX_EFF as _,
            txpwr_dft: esp_idf_sys::CONFIG_BT_CTRL_DFT_TX_POWER_LEVEL_EFF as _,
            cfg_mask: esp_idf_sys::CFG_MASK,
            scan_duplicate_mode: esp_idf_sys::SCAN_DUPLICATE_MODE as _,
            scan_duplicate_type: esp_idf_sys::SCAN_DUPLICATE_TYPE_VALUE as _,
            normal_adv_size: esp_idf_sys::NORMAL_SCAN_DUPLICATE_CACHE_SIZE as _,
            mesh_adv_size: esp_idf_sys::MESH_DUPLICATE_SCAN_CACHE_SIZE as _,
            coex_phy_coded_tx_rx_time_limit:
                esp_idf_sys::CONFIG_BT_CTRL_COEX_PHY_CODED_TX_RX_TLIM_EFF as _,
            hw_target_code: esp_idf_sys::BLE_HW_TARGET_CODE_CHIP_ECO0 as _,
            slave_ce_len_min: esp_idf_sys::SLAVE_CE_LEN_MIN_DEFAULT as _,
            hw_recorrect_en: esp_idf_sys::AGC_RECORRECT_EN as _,
            cca_thresh: esp_idf_sys::CONFIG_BT_CTRL_HW_CCA_VAL as _,
            coex_param_en: false,
            coex_use_hooks: false,
            ble_50_feat_supp: esp_idf_sys::BT_CTRL_50_FEATURE_SUPPORT != 0,
            dup_list_refresh_period: esp_idf_sys::DUPL_SCAN_CACHE_REFRESH_PERIOD as _,
            scan_backoff_upperlimitmax: esp_idf_sys::BT_CTRL_SCAN_BACKOFF_UPPERLIMITMAX as _,
        };

        #[cfg(esp32s3)]
        let mut bt_cfg = esp_bt_controller_config_t {
            magic: esp_idf_sys::ESP_BT_CTRL_CONFIG_MAGIC_VAL as _,
            version: esp_idf_sys::ESP_BT_CTRL_CONFIG_VERSION as _,
            controller_task_stack_size: esp_idf_sys::ESP_TASK_BT_CONTROLLER_STACK as _,
            controller_task_prio: esp_idf_sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
            controller_task_run_cpu: esp_idf_sys::CONFIG_BT_CTRL_PINNED_TO_CORE as _,
            bluetooth_mode: esp_idf_sys::CONFIG_BT_CTRL_MODE_EFF as _,
            ble_max_act: esp_idf_sys::CONFIG_BT_CTRL_BLE_MAX_ACT_EFF as _,
            sleep_mode: esp_idf_sys::CONFIG_BT_CTRL_SLEEP_MODE_EFF as _,
            sleep_clock: esp_idf_sys::CONFIG_BT_CTRL_SLEEP_CLOCK_EFF as _,
            ble_st_acl_tx_buf_nb: esp_idf_sys::CONFIG_BT_CTRL_BLE_STATIC_ACL_TX_BUF_NB as _,
            ble_hw_cca_check: esp_idf_sys::CONFIG_BT_CTRL_HW_CCA_EFF as _,
            ble_adv_dup_filt_max: esp_idf_sys::CONFIG_BT_CTRL_ADV_DUP_FILT_MAX as _,
            coex_param_en: false,
            ce_len_type: esp_idf_sys::CONFIG_BT_CTRL_CE_LENGTH_TYPE_EFF as _,
            coex_use_hooks: false,
            hci_tl_type: esp_idf_sys::CONFIG_BT_CTRL_HCI_TL_EFF as _,
            hci_tl_funcs: std::ptr::null_mut(),
            txant_dft: esp_idf_sys::CONFIG_BT_CTRL_TX_ANTENNA_INDEX_EFF as _,
            rxant_dft: esp_idf_sys::CONFIG_BT_CTRL_RX_ANTENNA_INDEX_EFF as _,
            txpwr_dft: esp_idf_sys::CONFIG_BT_CTRL_DFT_TX_POWER_LEVEL_EFF as _,
            cfg_mask: esp_idf_sys::CFG_MASK as _,
            scan_duplicate_mode: esp_idf_sys::SCAN_DUPLICATE_MODE as _,
            scan_duplicate_type: esp_idf_sys::SCAN_DUPLICATE_TYPE_VALUE as _,
            normal_adv_size: esp_idf_sys::NORMAL_SCAN_DUPLICATE_CACHE_SIZE as _,
            mesh_adv_size: esp_idf_sys::MESH_DUPLICATE_SCAN_CACHE_SIZE as _,
            coex_phy_coded_tx_rx_time_limit:
                esp_idf_sys::CONFIG_BT_CTRL_COEX_PHY_CODED_TX_RX_TLIM_EFF as _,
            #[cfg(esp_idf_version_major = "4")]
            hw_target_code: esp_idf_sys::BLE_HW_TARGET_CODE_ESP32S3_CHIP_ECO0 as _,
            #[cfg(not(esp_idf_version_major = "4"))]
            hw_target_code: esp_idf_sys::BLE_HW_TARGET_CODE_CHIP_ECO0 as _,
            slave_ce_len_min: esp_idf_sys::SLAVE_CE_LEN_MIN_DEFAULT as _,
            hw_recorrect_en: esp_idf_sys::AGC_RECORRECT_EN as _,
            cca_thresh: esp_idf_sys::CONFIG_BT_CTRL_HW_CCA_VAL as _,
            ..Default::default() // TODO
                                 // ble_50_feat_supp: esp_idf_sys::BT_CTRL_50_FEATURE_SUPPORT != 0,
                                 // dup_list_refresh_period: esp_idf_sys::DUPL_SCAN_CACHE_REFRESH_PERIOD as _,
                                 // scan_backoff_upperlimitmax: esp_idf_sys::BT_CTRL_SCAN_BACKOFF_UPPERLIMITMAX as _
        };

        info!("Init bluetooth controller.");
        esp!(unsafe { esp_bt_controller_init(&mut bt_cfg) })?;

        info!("Enable bluetooth controller.");
        esp!(unsafe { esp_bt_controller_enable(M::mode()) })?;

        info!("Init bluedroid");
        esp!(unsafe { esp_bluedroid_init() })?;

        info!("Enable bluedroid");
        esp!(unsafe { esp_bluedroid_enable() })?;

        // esp!(unsafe { esp_ble_gatts_register_callback(Some(gatts_event_handler)) })?;

        // esp!(unsafe { esp_ble_gap_register_callback(Some(gap_event_handler)) })?;

        // esp!(unsafe { esp_ble_gatt_set_local_mtu(500) })?;

        // let device_name_cstr = CString::new(device_name.clone()).unwrap();
        // esp!(unsafe { esp_ble_gap_set_device_name(device_name_cstr.as_ptr() as _) })?;

        Ok(())
    }
}

impl<'d, M> Drop for BtDriver<'d, M>
where
    M: BtMode,
{
    fn drop(&mut self) {
        let _ = esp!(unsafe { esp_bluedroid_disable() });

        esp!(unsafe { esp_bluedroid_deinit() }).unwrap();

        esp!(unsafe { esp_bt_controller_disable() }).unwrap();

        esp!(unsafe { esp_bt_controller_deinit() }).unwrap();
    }
}
