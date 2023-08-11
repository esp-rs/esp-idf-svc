use core::cell::UnsafeCell;
use core::fmt::{self, Debug};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};

use esp_idf_hal::modem::BluetoothModemPeripheral;
use esp_idf_hal::peripheral::Peripheral;

use esp_idf_sys::*;
use log::info;

#[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
use crate::nvs::EspDefaultNvsPartition;
use crate::private::cstr::to_cstring_arg;

extern crate alloc;

#[cfg(all(esp32, esp_idf_bt_classic_enabled, esp_idf_bt_a2dp_enable))]
pub mod a2dp;
pub mod ble;
#[cfg(all(esp32, esp_idf_bt_classic_enabled))]
pub mod gap;
#[cfg(all(esp32, esp_idf_bt_classic_enabled, esp_idf_bt_hfp_enable))]
pub mod hfp;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(transparent)]
pub struct BdAddr(esp_bd_addr_t);

impl BdAddr {
    pub fn raw(&self) -> esp_bd_addr_t {
        self.0
    }
}

impl From<BdAddr> for esp_bd_addr_t {
    fn from(value: BdAddr) -> Self {
        value.0
    }
}

impl From<esp_bd_addr_t> for BdAddr {
    fn from(value: esp_bd_addr_t) -> Self {
        Self(value)
    }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct BtUuid(esp_bt_uuid_t);

impl BtUuid {
    pub fn raw(&self) -> esp_bt_uuid_t {
        self.0
    }

    pub fn uuid16(uuid: u16) -> Self {
        let mut esp_uuid: esp_bt_uuid_t = Default::default();

        esp_uuid.len = 16;
        esp_uuid.uuid.uuid16 = uuid;

        Self(esp_uuid)
    }

    pub fn uuid32(uuid: u32) -> Self {
        let mut esp_uuid: esp_bt_uuid_t = Default::default();

        esp_uuid.len = 32;
        esp_uuid.uuid.uuid32 = uuid;

        Self(esp_uuid)
    }

    pub fn uuid128(uuid: u128) -> Self {
        let mut esp_uuid: esp_bt_uuid_t = Default::default();

        esp_uuid.len = 128;
        esp_uuid.uuid.uuid128 = uuid.to_ne_bytes();

        Self(esp_uuid)
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self.0.len {
            16 => unsafe {
                core::slice::from_raw_parts(&self.0.uuid.uuid128 as *const _ as *const _, 2)
            },
            32 => unsafe {
                core::slice::from_raw_parts(&self.0.uuid.uuid128 as *const _ as *const _, 4)
            },
            128 => unsafe { &self.0.uuid.uuid128 },
            _ => unreachable!(),
        }
    }
}

impl Debug for BtUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BtUuid {{{:?}}}", self.as_bytes())
    }
}

impl PartialEq for BtUuid {
    fn eq(&self, other: &BtUuid) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for BtUuid {}

impl From<BtUuid> for esp_bt_uuid_t {
    fn from(uuid: BtUuid) -> Self {
        uuid.0
    }
}

impl From<esp_bt_uuid_t> for BtUuid {
    fn from(uuid: esp_bt_uuid_t) -> Self {
        Self(uuid)
    }
}

pub(crate) struct BtCallback<A, R> {
    initialized: AtomicBool,
    callback: UnsafeCell<Option<alloc::boxed::Box<alloc::boxed::Box<dyn Fn(A) -> R>>>>,
    default_result: R,
}

impl<A, R> BtCallback<A, R>
where
    R: Clone,
{
    pub const fn new(default_result: R) -> Self {
        Self {
            initialized: AtomicBool::new(false),
            callback: UnsafeCell::new(None),
            default_result,
        }
    }

    pub fn set<'d, F>(&self, callback: F) -> Result<(), EspError>
    where
        F: Fn(A) -> R + Send + 'd,
    {
        self.initialized
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;

        let b: Box<dyn Fn(A) -> R + 'd> = Box::new(callback);
        let b: Box<dyn Fn(A) -> R + 'static> = unsafe { core::mem::transmute(b) };
        *unsafe { self.callback.get().as_mut() }.unwrap() = Some(Box::new(b));

        Ok(())
    }

    pub fn clear(&self) -> Result<(), EspError> {
        self.initialized
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;

        *unsafe { self.callback.get().as_mut() }.unwrap() = None;

        Ok(())
    }

    pub unsafe fn call(&self, arg: A) -> R {
        if let Some(callback) = unsafe { self.callback.get().as_ref() }.unwrap().as_ref() {
            (callback)(arg)
        } else {
            self.default_result.clone()
        }
    }
}

unsafe impl<A, R> Sync for BtCallback<A, R> {}
unsafe impl<A, R> Send for BtCallback<A, R> {}

pub trait BtMode {
    fn mode() -> esp_bt_mode_t;
}

pub trait BleEnabled: BtMode {}
pub trait BtClassicEnabled: BtMode {}

#[cfg(esp32)]
#[cfg(not(esp_idf_btdm_ctrl_mode_ble_only))]
pub struct BtClassic;
#[cfg(esp32)]
#[cfg(not(esp_idf_btdm_ctrl_mode_ble_only))]
impl BtClassicEnabled for BtClassic {}

#[cfg(esp32)]
#[cfg(not(esp_idf_btdm_ctrl_mode_ble_only))]
impl BtMode for BtClassic {
    fn mode() -> esp_bt_mode_t {
        #[cfg(not(esp_idf_btdm_ctrl_mode_btdm))]
        let mode = esp_bt_mode_t_ESP_BT_MODE_CLASSIC_BT;

        #[cfg(esp_idf_btdm_ctrl_mode_btdm)]
        let mode = esp_bt_mode_t_ESP_BT_MODE_BTDM;

        mode
    }
}

#[cfg(not(esp_idf_btdm_ctrl_mode_br_edr_only))]
pub struct Ble;
#[cfg(not(esp_idf_btdm_ctrl_mode_br_edr_only))]
impl BleEnabled for Ble {}

#[cfg(not(esp_idf_btdm_ctrl_mode_br_edr_only))]
impl BtMode for Ble {
    fn mode() -> esp_bt_mode_t {
        #[cfg(not(esp_idf_btdm_ctrl_mode_btdm))]
        let mode = esp_bt_mode_t_ESP_BT_MODE_BLE;

        #[cfg(esp_idf_btdm_ctrl_mode_btdm)]
        let mode = esp_bt_mode_t_ESP_BT_MODE_BTDM;

        mode
    }
}

#[cfg(esp32)]
#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
pub struct BtDual;
#[cfg(esp32)]
#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
impl BtClassicEnabled for BtDual {}
#[cfg(esp32)]
#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
impl BleEnabled for BtDual {}

#[cfg(esp32)]
#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
impl BtMode for BtDual {
    fn mode() -> esp_bt_mode_t {
        esp_bt_mode_t_ESP_BT_MODE_BTDM
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
            dup_list_refresh_period: SCAN_DUPL_CACHE_REFRESH_PERIOD as _,
            magic: ESP_BT_CONTROLLER_CONFIG_MAGIC_VAL,
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

        // ESP_ERROR_CHECK(esp_bt_controller_mem_release(ESP_BT_MODE_BLE));

        info!("Init bluetooth controller.");
        esp!(unsafe { esp_bt_controller_init(&mut bt_cfg) })?;

        info!("Enable bluetooth controller.");
        esp!(unsafe { esp_bt_controller_enable(M::mode()) })?;

        info!("Init bluedroid");
        esp!(unsafe { esp_bluedroid_init() })?;

        info!("Enable bluedroid");
        esp!(unsafe { esp_bluedroid_enable() })?;

        Ok(())
    }

    pub fn set_device_name(&self, device_name: &str) -> Result<(), EspError> {
        let device_name = to_cstring_arg(device_name)?;

        esp!(unsafe { esp_bt_dev_set_device_name(device_name.as_ptr()) })
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
