use core::cell::UnsafeCell;
use core::fmt::{self, Debug};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};

use log::info;

use num_enum::TryFromPrimitive;

use crate::hal::modem::BluetoothModemPeripheral;
use crate::hal::peripheral::Peripheral;

use crate::private::mutex;
use crate::sys::*;

#[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
use crate::nvs::EspDefaultNvsPartition;
use crate::private::cstr::to_cstring_arg;

extern crate alloc;

#[cfg(all(esp32, esp_idf_bt_classic_enabled, esp_idf_bt_a2dp_enable))]
pub mod a2dp;
#[cfg(all(esp32, esp_idf_bt_classic_enabled, esp_idf_bt_a2dp_enable))]
pub mod avrc;
// TODO: Future
// pub mod ble;
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
        let mut esp_uuid = esp_bt_uuid_t {
            len: 16,
            ..Default::default()
        };

        esp_uuid.uuid.uuid16 = uuid;

        Self(esp_uuid)
    }

    pub fn uuid32(uuid: u32) -> Self {
        let mut esp_uuid = esp_bt_uuid_t {
            len: 32,
            ..Default::default()
        };

        esp_uuid.uuid.uuid32 = uuid;

        Self(esp_uuid)
    }

    pub fn uuid128(uuid: u128) -> Self {
        let mut esp_uuid = esp_bt_uuid_t {
            len: 128,
            ..Default::default()
        };

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

#[allow(dead_code)]
#[allow(clippy::type_complexity)]
pub(crate) struct BtCallback<A, R> {
    initialized: AtomicBool,
    callback: UnsafeCell<Option<alloc::boxed::Box<dyn Fn(A) -> R>>>,
    default_result: R,
}

#[allow(dead_code)]
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

        let b: alloc::boxed::Box<dyn Fn(A) -> R + 'd> = alloc::boxed::Box::new(callback);
        let b: alloc::boxed::Box<dyn Fn(A) -> R + 'static> = unsafe { core::mem::transmute(b) };
        *unsafe { self.callback.get().as_mut() }.unwrap() = Some(b);

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
        if let Some(callback) = unsafe { self.callback.get().as_ref() }.unwrap() {
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

#[derive(Debug, Copy, Clone, Eq, PartialEq, TryFromPrimitive)]
#[repr(u32)]
pub enum BtStatus {
    Success = esp_bt_status_t_ESP_BT_STATUS_SUCCESS,
    Fail = esp_bt_status_t_ESP_BT_STATUS_FAIL,
    NotReady = esp_bt_status_t_ESP_BT_STATUS_NOT_READY,
    NoMem = esp_bt_status_t_ESP_BT_STATUS_NOMEM,
    Busy = esp_bt_status_t_ESP_BT_STATUS_BUSY,
    Done = esp_bt_status_t_ESP_BT_STATUS_DONE,
    Unsupported = esp_bt_status_t_ESP_BT_STATUS_UNSUPPORTED,
    InvalidParam = esp_bt_status_t_ESP_BT_STATUS_PARM_INVALID,
    Unhandled = esp_bt_status_t_ESP_BT_STATUS_UNHANDLED,
    AuthFailure = esp_bt_status_t_ESP_BT_STATUS_AUTH_FAILURE,
    RemoteDeviceDown = esp_bt_status_t_ESP_BT_STATUS_RMT_DEV_DOWN,
    AuthRejected = esp_bt_status_t_ESP_BT_STATUS_AUTH_REJECTED,
    InvalidStaticRandAddr = esp_bt_status_t_ESP_BT_STATUS_INVALID_STATIC_RAND_ADDR,
    Pending = esp_bt_status_t_ESP_BT_STATUS_PENDING,
    UnacceptedConnInterval = esp_bt_status_t_ESP_BT_STATUS_UNACCEPT_CONN_INTERVAL,
    ParamOutOfRange = esp_bt_status_t_ESP_BT_STATUS_PARAM_OUT_OF_RANGE,
    Timeout = esp_bt_status_t_ESP_BT_STATUS_TIMEOUT,
    UnsupportedPeerLeDataLen = esp_bt_status_t_ESP_BT_STATUS_PEER_LE_DATA_LEN_UNSUPPORTED,
    UnsupportedControlLeDataLen = esp_bt_status_t_ESP_BT_STATUS_CONTROL_LE_DATA_LEN_UNSUPPORTED,
    IllegalParamFormat = esp_bt_status_t_ESP_BT_STATUS_ERR_ILLEGAL_PARAMETER_FMT,
    MemoryFull = esp_bt_status_t_ESP_BT_STATUS_MEMORY_FULL,
    EirTooLarge = esp_bt_status_t_ESP_BT_STATUS_EIR_TOO_LARGE,
    HciSuccess = esp_bt_status_t_ESP_BT_STATUS_HCI_SUCCESS,
    HciIllegalCommand = esp_bt_status_t_ESP_BT_STATUS_HCI_ILLEGAL_COMMAND,
    HciNoConnection = esp_bt_status_t_ESP_BT_STATUS_HCI_NO_CONNECTION,
    HciHwFailure = esp_bt_status_t_ESP_BT_STATUS_HCI_HW_FAILURE,
    HciPageTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_PAGE_TIMEOUT,
    HciAuthFailure = esp_bt_status_t_ESP_BT_STATUS_HCI_AUTH_FAILURE,
    HciKeyMissing = esp_bt_status_t_ESP_BT_STATUS_HCI_KEY_MISSING,
    HciMemoryFull = esp_bt_status_t_ESP_BT_STATUS_HCI_MEMORY_FULL,
    HciConnTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_CONNECTION_TOUT,
    HciConnectionsExhausted = esp_bt_status_t_ESP_BT_STATUS_HCI_MAX_NUM_OF_CONNECTIONS,
    HciScosExhausted = esp_bt_status_t_ESP_BT_STATUS_HCI_MAX_NUM_OF_SCOS,
    HciConnectionExists = esp_bt_status_t_ESP_BT_STATUS_HCI_CONNECTION_EXISTS,
    HciCommandDisallowed = esp_bt_status_t_ESP_BT_STATUS_HCI_COMMAND_DISALLOWED,
    HciHostResourcesRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_REJECT_RESOURCES,
    HciHostSecurityRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_REJECT_SECURITY,
    HciHostDevideRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_REJECT_DEVICE,
    HciHostTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_TIMEOUT,
    HciUnsupportedValue = esp_bt_status_t_ESP_BT_STATUS_HCI_UNSUPPORTED_VALUE,
    HciIllegalParamFormat = esp_bt_status_t_ESP_BT_STATUS_HCI_ILLEGAL_PARAMETER_FMT,
    HciPeerUser = esp_bt_status_t_ESP_BT_STATUS_HCI_PEER_USER,
    HciPeerLowResources = esp_bt_status_t_ESP_BT_STATUS_HCI_PEER_LOW_RESOURCES,
    HciPeerPowerOff = esp_bt_status_t_ESP_BT_STATUS_HCI_PEER_POWER_OFF,
    HciConnectionCauseLocalHost = esp_bt_status_t_ESP_BT_STATUS_HCI_CONN_CAUSE_LOCAL_HOST,
    HciRepeatedAttempts = esp_bt_status_t_ESP_BT_STATUS_HCI_REPEATED_ATTEMPTS,
    HciPairingNotAllowed = esp_bt_status_t_ESP_BT_STATUS_HCI_PAIRING_NOT_ALLOWED,
    HciUnkownLmpPdu = esp_bt_status_t_ESP_BT_STATUS_HCI_UNKNOWN_LMP_PDU,
    HciUnsupportedRemFeature = esp_bt_status_t_ESP_BT_STATUS_HCI_UNSUPPORTED_REM_FEATURE,
    HciScoOffsetRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_SCO_OFFSET_REJECTED,
    HciScoInternalRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_SCO_INTERVAL_REJECTED,
    HciScoAirMode = esp_bt_status_t_ESP_BT_STATUS_HCI_SCO_AIR_MODE,
    HciInvalidLmpParam = esp_bt_status_t_ESP_BT_STATUS_HCI_INVALID_LMP_PARAM,
    HciUnspecified = esp_bt_status_t_ESP_BT_STATUS_HCI_UNSPECIFIED,
    HciUnsupportedLmpParameters = esp_bt_status_t_ESP_BT_STATUS_HCI_UNSUPPORTED_LMP_PARAMETERS,
    HciRoleChangeNotAllowed = esp_bt_status_t_ESP_BT_STATUS_HCI_ROLE_CHANGE_NOT_ALLOWED,
    HciLmpResponseTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_LMP_RESPONSE_TIMEOUT,
    HciLmpErrTransactionCollision = esp_bt_status_t_ESP_BT_STATUS_HCI_LMP_ERR_TRANS_COLLISION,
    HciLmpPduNotAllowed = esp_bt_status_t_ESP_BT_STATUS_HCI_LMP_PDU_NOT_ALLOWED,
    HciEntryModeNotAcceptable = esp_bt_status_t_ESP_BT_STATUS_HCI_ENCRY_MODE_NOT_ACCEPTABLE,
    HciUnitKeyUsed = esp_bt_status_t_ESP_BT_STATUS_HCI_UNIT_KEY_USED,
    HciUnsupportedQos = esp_bt_status_t_ESP_BT_STATUS_HCI_QOS_NOT_SUPPORTED,
    HciInstantPassed = esp_bt_status_t_ESP_BT_STATUS_HCI_INSTANT_PASSED,
    HciUnsupportedPairingWithUnitKey =
        esp_bt_status_t_ESP_BT_STATUS_HCI_PAIRING_WITH_UNIT_KEY_NOT_SUPPORTED,
    HciDiffTransactionCollision = esp_bt_status_t_ESP_BT_STATUS_HCI_DIFF_TRANSACTION_COLLISION,
    HciUndefined0x2b = esp_bt_status_t_ESP_BT_STATUS_HCI_UNDEFINED_0x2B,
    HciQosInvalidParam = esp_bt_status_t_ESP_BT_STATUS_HCI_QOS_UNACCEPTABLE_PARAM,
    HciQosRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_QOS_REJECTED,
    HciUnsupportedChanClassification = esp_bt_status_t_ESP_BT_STATUS_HCI_CHAN_CLASSIF_NOT_SUPPORTED,
    HciInsufficientSecurity = esp_bt_status_t_ESP_BT_STATUS_HCI_INSUFFCIENT_SECURITY,
    HciParamOutOfRange = esp_bt_status_t_ESP_BT_STATUS_HCI_PARAM_OUT_OF_RANGE,
    HciUndefined0x31 = esp_bt_status_t_ESP_BT_STATUS_HCI_UNDEFINED_0x31,
    HciRoleSwitchPending = esp_bt_status_t_ESP_BT_STATUS_HCI_ROLE_SWITCH_PENDING,
    HciUndefined0x33 = esp_bt_status_t_ESP_BT_STATUS_HCI_UNDEFINED_0x33,
    HciReservedSlotViolation = esp_bt_status_t_ESP_BT_STATUS_HCI_RESERVED_SLOT_VIOLATION,
    HciRoleSwitchFailed = esp_bt_status_t_ESP_BT_STATUS_HCI_ROLE_SWITCH_FAILED,
    HciInqRespDataTooLarge = esp_bt_status_t_ESP_BT_STATUS_HCI_INQ_RSP_DATA_TOO_LARGE,
    HciSimplePairingNotSupported = esp_bt_status_t_ESP_BT_STATUS_HCI_SIMPLE_PAIRING_NOT_SUPPORTED,
    HciHostBusyPairing = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_BUSY_PAIRING,
    HciRejNoSuitableChannel = esp_bt_status_t_ESP_BT_STATUS_HCI_REJ_NO_SUITABLE_CHANNEL,
    HciControllerBusy = esp_bt_status_t_ESP_BT_STATUS_HCI_CONTROLLER_BUSY,
    HciUnsupportedConnectionInterval = esp_bt_status_t_ESP_BT_STATUS_HCI_UNACCEPT_CONN_INTERVAL,
    HciDirectedAdvertisingTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_DIRECTED_ADVERTISING_TIMEOUT,
    HciConnectionTimeoutDueToMiscFailure =
        esp_bt_status_t_ESP_BT_STATUS_HCI_CONN_TOUT_DUE_TO_MIC_FAILURE,
    HciConnectionEstablishmentFailed = esp_bt_status_t_ESP_BT_STATUS_HCI_CONN_FAILED_ESTABLISHMENT,
    HciMacConnectionFailed = esp_bt_status_t_ESP_BT_STATUS_HCI_MAC_CONNECTION_FAILED,
}

static MEM_FREED: mutex::Mutex<bool> = mutex::Mutex::new(false);

pub fn reduce_bt_memory<'d, B: BluetoothModemPeripheral>(
    _modem: impl Peripheral<P = B> + 'd,
) -> Result<(), EspError> {
    let mut mem_freed = MEM_FREED.lock();

    if *mem_freed {
        Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;
    }

    #[cfg(esp_idf_btdm_ctrl_mode_br_edr_only)]
    esp!(unsafe { esp_bt_mem_release(esp_bt_mode_t_ESP_BT_MODE_BLE) })?;

    #[cfg(esp_idf_btdm_ctrl_mode_br_ble_only)]
    esp!(unsafe { esp_bt_mem_release(esp_bt_mode_t_ESP_BT_MODE_CLASSIC_BT) })?;

    *mem_freed = true;

    Ok(())
}

pub fn free_bt_memory<B: BluetoothModemPeripheral>(_modem: B) -> Result<(), EspError> {
    let mut mem_freed = MEM_FREED.lock();

    if *mem_freed {
        Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;
    }

    esp!(unsafe { esp_bt_mem_release(esp_bt_mode_t_ESP_BT_MODE_BTDM) })?;

    *mem_freed = true;

    Ok(())
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

    #[allow(clippy::needless_update)]
    fn init(_nvs_enabled: bool) -> Result<(), EspError> {
        #[cfg(esp32)]
        let mut bt_cfg = esp_bt_controller_config_t {
            magic: ESP_BT_CONTROLLER_CONFIG_MAGIC_VAL,
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
            ..Default::default()
        };

        #[cfg(esp32c3)]
        let mut bt_cfg = esp_bt_controller_config_t {
            magic: crate::sys::ESP_BT_CTRL_CONFIG_MAGIC_VAL,
            version: crate::sys::ESP_BT_CTRL_CONFIG_VERSION,
            controller_task_stack_size: crate::sys::ESP_TASK_BT_CONTROLLER_STACK as _,
            controller_task_prio: crate::sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
            controller_task_run_cpu: crate::sys::CONFIG_BT_CTRL_PINNED_TO_CORE as _,
            bluetooth_mode: crate::sys::CONFIG_BT_CTRL_MODE_EFF as _,
            ble_max_act: crate::sys::CONFIG_BT_CTRL_BLE_MAX_ACT_EFF as _,
            sleep_mode: crate::sys::CONFIG_BT_CTRL_SLEEP_MODE_EFF as _,
            sleep_clock: crate::sys::CONFIG_BT_CTRL_SLEEP_CLOCK_EFF as _,
            ble_st_acl_tx_buf_nb: crate::sys::CONFIG_BT_CTRL_BLE_STATIC_ACL_TX_BUF_NB as _,
            ble_hw_cca_check: crate::sys::CONFIG_BT_CTRL_HW_CCA_EFF as _,
            ble_adv_dup_filt_max: crate::sys::CONFIG_BT_CTRL_ADV_DUP_FILT_MAX as _,
            ce_len_type: crate::sys::CONFIG_BT_CTRL_CE_LENGTH_TYPE_EFF as _,
            hci_tl_type: crate::sys::CONFIG_BT_CTRL_HCI_TL_EFF as _,
            hci_tl_funcs: core::ptr::null_mut(),
            txant_dft: crate::sys::CONFIG_BT_CTRL_TX_ANTENNA_INDEX_EFF as _,
            rxant_dft: crate::sys::CONFIG_BT_CTRL_RX_ANTENNA_INDEX_EFF as _,
            txpwr_dft: crate::sys::CONFIG_BT_CTRL_DFT_TX_POWER_LEVEL_EFF as _,
            cfg_mask: crate::sys::CFG_MASK,
            scan_duplicate_mode: crate::sys::SCAN_DUPLICATE_MODE as _,
            scan_duplicate_type: crate::sys::SCAN_DUPLICATE_TYPE_VALUE as _,
            normal_adv_size: crate::sys::NORMAL_SCAN_DUPLICATE_CACHE_SIZE as _,
            mesh_adv_size: crate::sys::MESH_DUPLICATE_SCAN_CACHE_SIZE as _,
            coex_phy_coded_tx_rx_time_limit:
                crate::sys::CONFIG_BT_CTRL_COEX_PHY_CODED_TX_RX_TLIM_EFF as _,
            hw_target_code: crate::sys::BLE_HW_TARGET_CODE_CHIP_ECO0 as _,
            slave_ce_len_min: crate::sys::SLAVE_CE_LEN_MIN_DEFAULT as _,
            hw_recorrect_en: crate::sys::AGC_RECORRECT_EN as _,
            cca_thresh: crate::sys::CONFIG_BT_CTRL_HW_CCA_VAL as _,
            coex_param_en: false,
            coex_use_hooks: false,
            ble_50_feat_supp: crate::sys::BT_CTRL_50_FEATURE_SUPPORT != 0,
            dup_list_refresh_period: crate::sys::DUPL_SCAN_CACHE_REFRESH_PERIOD as _,
            scan_backoff_upperlimitmax: crate::sys::BT_CTRL_SCAN_BACKOFF_UPPERLIMITMAX as _,
            ..Default::default()
        };

        #[cfg(esp32s3)]
        let mut bt_cfg = esp_bt_controller_config_t {
            magic: crate::sys::ESP_BT_CTRL_CONFIG_MAGIC_VAL as _,
            version: crate::sys::ESP_BT_CTRL_CONFIG_VERSION as _,
            controller_task_stack_size: crate::sys::ESP_TASK_BT_CONTROLLER_STACK as _,
            controller_task_prio: crate::sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
            controller_task_run_cpu: crate::sys::CONFIG_BT_CTRL_PINNED_TO_CORE as _,
            bluetooth_mode: crate::sys::CONFIG_BT_CTRL_MODE_EFF as _,
            ble_max_act: crate::sys::CONFIG_BT_CTRL_BLE_MAX_ACT_EFF as _,
            sleep_mode: crate::sys::CONFIG_BT_CTRL_SLEEP_MODE_EFF as _,
            sleep_clock: crate::sys::CONFIG_BT_CTRL_SLEEP_CLOCK_EFF as _,
            ble_st_acl_tx_buf_nb: crate::sys::CONFIG_BT_CTRL_BLE_STATIC_ACL_TX_BUF_NB as _,
            ble_hw_cca_check: crate::sys::CONFIG_BT_CTRL_HW_CCA_EFF as _,
            ble_adv_dup_filt_max: crate::sys::CONFIG_BT_CTRL_ADV_DUP_FILT_MAX as _,
            coex_param_en: false,
            ce_len_type: crate::sys::CONFIG_BT_CTRL_CE_LENGTH_TYPE_EFF as _,
            coex_use_hooks: false,
            hci_tl_type: crate::sys::CONFIG_BT_CTRL_HCI_TL_EFF as _,
            hci_tl_funcs: core::ptr::null_mut(),
            txant_dft: crate::sys::CONFIG_BT_CTRL_TX_ANTENNA_INDEX_EFF as _,
            rxant_dft: crate::sys::CONFIG_BT_CTRL_RX_ANTENNA_INDEX_EFF as _,
            txpwr_dft: crate::sys::CONFIG_BT_CTRL_DFT_TX_POWER_LEVEL_EFF as _,
            cfg_mask: crate::sys::CFG_MASK as _,
            scan_duplicate_mode: crate::sys::SCAN_DUPLICATE_MODE as _,
            scan_duplicate_type: crate::sys::SCAN_DUPLICATE_TYPE_VALUE as _,
            normal_adv_size: crate::sys::NORMAL_SCAN_DUPLICATE_CACHE_SIZE as _,
            mesh_adv_size: crate::sys::MESH_DUPLICATE_SCAN_CACHE_SIZE as _,
            coex_phy_coded_tx_rx_time_limit:
                crate::sys::CONFIG_BT_CTRL_COEX_PHY_CODED_TX_RX_TLIM_EFF as _,
            hw_target_code: crate::sys::BLE_HW_TARGET_CODE_CHIP_ECO0 as _,
            slave_ce_len_min: crate::sys::SLAVE_CE_LEN_MIN_DEFAULT as _,
            hw_recorrect_en: crate::sys::AGC_RECORRECT_EN as _,
            cca_thresh: crate::sys::CONFIG_BT_CTRL_HW_CCA_VAL as _,
            ..Default::default() // TODO
                                 // ble_50_feat_supp: crate::sys::BT_CTRL_50_FEATURE_SUPPORT != 0,
                                 // dup_list_refresh_period: crate::sys::DUPL_SCAN_CACHE_REFRESH_PERIOD as _,
                                 // scan_backoff_upperlimitmax: crate::sys::BT_CTRL_SCAN_BACKOFF_UPPERLIMITMAX as _
        };

        info!("Init bluetooth controller");
        esp!(unsafe { esp_bt_controller_init(&mut bt_cfg) })?;

        info!("Enable bluetooth controller");
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
