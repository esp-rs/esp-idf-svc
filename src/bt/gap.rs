use core::cmp::min;
use core::{borrow::Borrow, fmt::Debug, marker::PhantomData};

use enumset::{EnumSet, EnumSetType};

use esp_idf_sys::*;

use log::{debug, info};

use crate::bt::BtCallback;
use crate::bt::{BleEnabled, BtDriver, BtUuid};

use super::BdAddr;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(u32)]
pub enum DiscoveryMode {
    NonDiscoverable = esp_bt_discovery_mode_t_ESP_BT_NON_DISCOVERABLE,
    Limited = esp_bt_discovery_mode_t_ESP_BT_LIMITED_DISCOVERABLE,
    Discoverable = esp_bt_discovery_mode_t_ESP_BT_GENERAL_DISCOVERABLE,
}

// impl From<DiscoveryMode> for esp_bt_discovery_mode_t {
//     fn from(value: DiscoveryMode) -> Self {
//         match self {

//         }
//     }
// }

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(u32)]
pub enum InqMode {
    General = esp_bt_inq_mode_t_ESP_BT_INQ_MODE_GENERAL_INQUIRY,
    Limited = esp_bt_inq_mode_t_ESP_BT_INQ_MODE_LIMITED_INQUIRY,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct EirType(esp_bt_eir_type_t);

impl EirType {
    pub fn raw(&self) -> esp_bt_eir_type_t {
        self.0
    }
}

impl From<EirType> for esp_bt_eir_type_t {
    fn from(value: EirType) -> Self {
        value.0
    }
}

impl From<esp_bt_eir_type_t> for EirType {
    fn from(value: esp_bt_eir_type_t) -> Self {
        Self(value)
    }
}

#[derive(Debug, EnumSetType)]
#[enumset(repr = "u8")]
#[repr(u8)]
pub enum EirFlags {
    LimitDisc = 0,
    GenDisc = 1,
    BredrNotSpt = 2,
    DmtControllerSpt = 3,
    DmtHostSpt = 4,
}

#[derive(Debug, Clone)]
pub struct EirData<'a> {
    pub fec_required: bool,       // FEC is required or not, true by default
    pub include_txpower: bool,    // EIR data include TX power, false by default
    pub include_uuid: bool,       // EIR data include UUID, false by default
    pub flags: EnumSet<EirFlags>, // EIR flags, see ESP_BT_EIR_FLAG for details, EIR will not include flag if it is 0, 0 by default
    pub manufacturer: &'a [u8],   // Manufacturer data point
    pub url: &'a str,             // URL point
}

impl<'a> From<&EirData<'a>> for esp_bt_eir_data_t {
    fn from(data: &EirData<'a>) -> Self {
        Self {
            fec_required: data.fec_required,
            include_txpower: data.include_txpower,
            include_uuid: data.include_uuid,
            flag: data.flags.as_repr(),
            p_manufacturer_data: data.manufacturer.as_ptr() as *mut _,
            manufacturer_len: data.manufacturer.len() as _,
            p_url: data.url.as_ptr() as *mut _,
            url_len: data.url.len() as _,
        }
    }
}

impl<'a> From<&esp_bt_eir_data_t> for EirData<'a> {
    fn from(data: &esp_bt_eir_data_t) -> Self {
        Self {
            fec_required: data.fec_required,
            include_txpower: data.include_txpower,
            include_uuid: data.include_uuid,
            flags: EnumSet::from_repr(data.flag),
            manufacturer: unsafe {
                core::slice::from_raw_parts(
                    data.p_manufacturer_data as *mut u8,
                    data.manufacturer_len as _,
                )
            },
            url: unsafe {
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                    data.p_url as *mut u8,
                    data.url_len as _,
                ))
            },
        }
    }
}

#[derive(Debug, EnumSetType)]
#[enumset(repr = "u32")]
#[repr(u32)]
pub enum CodMode {
    SetMajorMinor = 1,   // overwrite major, minor class
    SetServiceClass = 2, // set the bits in the input, the current bit will remain
    ClrServiceClass = 4, // clear the bits in the input, others will remain
    SetAll = 8,          // overwrite major, minor, set the bits in service class
    Init = 10,           // overwrite major, minor, and service class
}

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct Cod(esp_bt_cod_t);

impl Cod {
    pub fn new(major: u32, minor: u32, service: u32) -> Self {
        let mut cod: esp_bt_cod_t = Default::default();

        cod.set_minor(minor);
        cod.set_major(major);
        cod.set_service(service);

        Self(cod)
    }

    pub fn raw(&self) -> esp_bt_cod_t {
        self.0
    }

    pub fn major(&self) -> u32 {
        self.0.major()
    }

    pub fn minor(&self) -> u32 {
        self.0.minor()
    }

    pub fn service(&self) -> u32 {
        self.0.service()
    }
}

impl From<Cod> for esp_bt_cod_t {
    fn from(cod: Cod) -> Self {
        cod.0
    }
}

impl From<esp_bt_cod_t> for Cod {
    fn from(cod: esp_bt_cod_t) -> Self {
        Self(cod)
    }
}

impl PartialEq for Cod {
    fn eq(&self, other: &Self) -> bool {
        self.0.major() == other.0.major()
            && self.0.minor() == other.0.minor()
            && self.0.service() == other.0.service()
    }
}

impl Eq for Cod {}

#[derive(Debug)]
pub enum DeviceProp<'a> {
    BdName(&'a str),
    Cod(Cod),
    Rssi(u8),
    EirData(EirData<'a>),
}

#[derive(Clone, Debug)]
#[repr(transparent)]
pub struct PropData<'a> {
    data: esp_bt_gap_dev_prop_t,
    _p: PhantomData<&'a ()>,
}

#[allow(non_upper_case_globals)]
impl<'a> PropData<'a> {
    pub fn prop(&self) -> DeviceProp {
        unsafe {
            match self.data.type_ {
                esp_bt_gap_dev_prop_type_t_ESP_BT_GAP_DEV_PROP_BDNAME => {
                    DeviceProp::BdName(core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                        self.data.val as *mut u8 as *const _,
                        self.data.len as _,
                    )))
                }
                esp_bt_gap_dev_prop_type_t_ESP_BT_GAP_DEV_PROP_COD => {
                    let esp_cod = (self.data.val as *mut esp_bt_cod_t).as_ref().unwrap();
                    DeviceProp::Cod((*esp_cod).into())
                }
                esp_bt_gap_dev_prop_type_t_ESP_BT_GAP_DEV_PROP_RSSI => {
                    let rssi = (self.data.val as *mut u8).as_ref().unwrap();
                    DeviceProp::Rssi(*rssi)
                }
                esp_bt_gap_dev_prop_type_t_ESP_BT_GAP_DEV_PROP_EIR => {
                    let esp_eir_data = (self.data.val as *mut esp_bt_eir_data_t).as_ref().unwrap();
                    DeviceProp::EirData(esp_eir_data.into())
                }
                _ => unreachable!(),
            }
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(u32)]
pub enum BtStatus {
    // TODO
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
    HciPending = esp_bt_status_t_ESP_BT_STATUS_HCI_PENDING,
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

impl From<esp_bt_status_t> for BtStatus {
    fn from(value: esp_bt_status_t) -> Self {
        unsafe { core::mem::transmute(value) } // TODO
    }
}

impl From<BtStatus> for esp_bt_status_t {
    fn from(value: BtStatus) -> Self {
        value as _
    }
}

#[derive(Clone, Debug)]
pub enum GapEvent<'a> {
    DeviceDiscoveryStarted,
    DeviceDiscoveryStopped,
    DeviceDiscovered {
        bda: BdAddr,
        props: &'a [PropData<'a>],
    },
    RemoteServiceDiscovered {
        bda: BdAddr,
        status: BtStatus,
        services: &'a [BtUuid],
    },
    RemoteServiceDetails {
        bda: BdAddr,
        status: BtStatus,
    },
    AuthenticationCompleted {
        bda: BdAddr,
        status: BtStatus,
        device_name: &'a str,
    },
    PairingPinRequest {
        bda: BdAddr,
        min_16_digit: bool,
    },
    PairingUserConfirmationRequest {
        bda: BdAddr,
        number: u32,
    },
    PairingPasskeyRequest {
        bda: BdAddr,
    },
    PairingPasskey {
        bda: BdAddr,
        passkey: u32,
    },
    Rssi {
        bda: BdAddr,
        status: BtStatus,
        rssi: i8,
    },
    EirDataConfigured {
        status: BtStatus,
        eir_types: &'a [EirType],
    },
    AfhChannelsConfigured {
        status: BtStatus,
    },
    RemoteName {
        bda: BdAddr,
        status: BtStatus,
        name: &'a str,
    },
    ModeChange {
        bda: BdAddr,
        mode: u8,
    },
    BondedServiceRemoved {
        bda: BdAddr,
        status: BtStatus,
    },
    AclConnected {
        bda: BdAddr,
        status: BtStatus,
        handle: u16,
    },
    AclDisconnected {
        bda: BdAddr,
        status: BtStatus,
        handle: u16,
    },
    QosComplete {
        bda: BdAddr,
        status: BtStatus,
        t_poll: u32,
    },
    Other,
}

#[allow(non_upper_case_globals)]
impl<'a> From<(esp_bt_gap_cb_event_t, &'a esp_bt_gap_cb_param_t)> for GapEvent<'a> {
    fn from(value: (esp_bt_gap_cb_event_t, &'a esp_bt_gap_cb_param_t)) -> Self {
        let (evt, param) = value;

        unsafe {
            match evt {
                esp_bt_gap_cb_event_t_ESP_BT_GAP_DISC_RES_EVT => Self::DeviceDiscovered {
                    bda: param.disc_res.bda.into(),
                    props: core::slice::from_raw_parts(
                        param.disc_res.prop as *mut PropData as *const _,
                        param.disc_res.num_prop as _,
                    ),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_DISC_STATE_CHANGED_EVT => {
                    if param.disc_st_chg.state
                        == esp_bt_gap_discovery_state_t_ESP_BT_GAP_DISCOVERY_STOPPED
                    {
                        Self::DeviceDiscoveryStarted
                    } else {
                        Self::DeviceDiscoveryStopped
                    }
                }
                esp_bt_gap_cb_event_t_ESP_BT_GAP_RMT_SRVCS_EVT => Self::RemoteServiceDiscovered {
                    bda: param.rmt_srvcs.bda.into(),
                    status: param.rmt_srvcs.stat.into(),
                    services: core::slice::from_raw_parts(
                        param.rmt_srvcs.uuid_list as *mut BtUuid as *const _,
                        param.rmt_srvcs.num_uuids as _,
                    ),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_RMT_SRVC_REC_EVT => Self::RemoteServiceDetails {
                    bda: param.rmt_srvcs.bda.into(),
                    status: param.rmt_srvcs.stat.into(),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_AUTH_CMPL_EVT => Self::AuthenticationCompleted {
                    bda: param.auth_cmpl.bda.into(),
                    status: param.auth_cmpl.stat.into(),
                    device_name: core::str::from_utf8_unchecked(
                        &param.auth_cmpl.device_name
                            [..strlen(&param.auth_cmpl.device_name as *const _ as *const _) as _],
                    ),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_PIN_REQ_EVT => Self::PairingPinRequest {
                    bda: param.pin_req.bda.into(),
                    min_16_digit: param.pin_req.min_16_digit,
                },

                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_DATA_SET_COMPLETE_EVT => {
                //         Self::AdvertisingDatasetComplete(param.adv_data_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SCAN_RSP_DATA_SET_COMPLETE_EVT => {
                //         Self::ScanResponseDatasetComplete(param.scan_rsp_data_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SCAN_PARAM_SET_COMPLETE_EVT => {
                //         Self::ScanParameterDatasetComplete(param.scan_param_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SCAN_RESULT_EVT => {
                //         Self::ScanResult(param.scan_rst)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_DATA_RAW_SET_COMPLETE_EVT => {
                //         Self::RawAdvertisingDatasetComplete(param.adv_data_raw_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SCAN_RSP_DATA_RAW_SET_COMPLETE_EVT => {
                //         Self::RawScanResponseDatasetComplete(param.scan_rsp_data_raw_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_START_COMPLETE_EVT => {
                //         Self::AdvertisingStartComplete(param.adv_start_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SCAN_START_COMPLETE_EVT => {
                //         Self::ScanStartComplete(param.scan_start_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_AUTH_CMPL_EVT => {
                //         Self::AuthenticationComplete(param.ble_security)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_KEY_EVT => Self::Key(param.ble_security),
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SEC_REQ_EVT => {
                //         Self::SecurityRequest(param.ble_security)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_PASSKEY_NOTIF_EVT => {
                //         Self::PasskeyNotification(param.ble_security)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_PASSKEY_REQ_EVT => {
                //         Self::PasskeyRequest(param.ble_security)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_OOB_REQ_EVT => Self::OOBRequest,
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_LOCAL_IR_EVT => Self::LocalIR,
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_LOCAL_ER_EVT => Self::LocalER,
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_NC_REQ_EVT => {
                //         Self::NumericComparisonRequest(param.ble_security)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_ADV_STOP_COMPLETE_EVT => {
                //         Self::AdvertisingStopComplete(param.adv_stop_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SCAN_STOP_COMPLETE_EVT => {
                //         Self::ScanStopComplete(param.scan_stop_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SET_STATIC_RAND_ADDR_EVT => {
                //         Self::SetStaticRandomAddressComplete(param.set_rand_addr_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_UPDATE_CONN_PARAMS_EVT => {
                //         Self::UpdateConnectionParamsComplete(param.update_conn_params)
                //     }
                //     #[cfg(esp_idf_version_major = "4")]
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SET_PKT_LENGTH_COMPLETE_EVT => {
                //         Self::SetPacketLengthComplete(param.pkt_data_lenth_cmpl)
                //     }
                //     #[cfg(not(esp_idf_version_major = "4"))]
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SET_PKT_LENGTH_COMPLETE_EVT => {
                //         Self::SetPacketLengthComplete(param.pkt_data_length_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SET_LOCAL_PRIVACY_COMPLETE_EVT => {
                //         Self::SetLocalPrivacy(param.local_privacy_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_REMOVE_BOND_DEV_COMPLETE_EVT => {
                //         Self::RemoveDeviceBondComplete(param.remove_bond_dev_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_CLEAR_BOND_DEV_COMPLETE_EVT => {
                //         Self::ClearDeviceBondComplete(param.clear_bond_dev_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_GET_BOND_DEV_COMPLETE_EVT => {
                //         Self::GetDeviceBondComplete(param.get_bond_dev_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_READ_RSSI_COMPLETE_EVT => {
                //         Self::ReadRssiComplete(param.read_rssi_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_UPDATE_WHITELIST_COMPLETE_EVT => {
                //         Self::UpdateWhitelistComplete(param.update_whitelist_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_UPDATE_DUPLICATE_EXCEPTIONAL_LIST_COMPLETE_EVT => {
                //         Self::UpdateDuplicateListComplete(param.update_duplicate_exceptional_list_cmpl)
                //     }
                //     esp_gap_ble_cb_event_t_ESP_GAP_BLE_SET_CHANNELS_EVT => {
                //         Self::SetChannelsComplete(param.ble_set_channels)
                //     }
                _ => {
                    log::warn!("Unhandled event {:?}", evt);
                    Self::Other
                    //panic!("Unhandled event {:?}", evt)
                }
            }
        }
    }
}

// impl Debug for GapEvent {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
//         write!(
//             f,
//             "{}",
//             match self {
//                 Self::AdvertisingDatasetComplete(_) => "AdvertisingDatasetComplete",
//                 Self::ScanResponseDatasetComplete(_) => "ScanResponseDatasetComplete",
//                 Self::ScanParameterDatasetComplete(_) => "ScanParameterDatasetComplete",
//                 Self::ScanResult(_) => "ScanResult",
//                 Self::RawAdvertisingDatasetComplete(_) => "RawAdvertisingDatasetComplete",
//                 Self::RawScanResponseDatasetComplete(_) => "RawScanResponseDatasetComplete",
//                 Self::AdvertisingStartComplete(_) => "AdvertisingStartComplete",
//                 Self::ScanStartComplete(_) => "ScanStartComplete",
//                 Self::AuthenticationComplete(_) => "AuthenticationComplete",
//                 Self::Key(_) => "Key",
//                 Self::SecurityRequest(_) => "SecurityRequest",
//                 Self::PasskeyNotification(_) => "PasskeyNotification",
//                 Self::PasskeyRequest(_) => "PasskeyRequest",
//                 Self::OOBRequest => "OOBRequest",
//                 Self::LocalIR => "LocalIR",
//                 Self::LocalER => "LocalER",
//                 Self::NumericComparisonRequest(_) => "NumericComparisonRequest",
//                 Self::AdvertisingStopComplete(_) => "AdvertisingStopComplete",
//                 Self::ScanStopComplete(_) => "ScanStopComplete",
//                 Self::SetStaticRandomAddressComplete(_) => "SetStaticRandomAddressComplete",
//                 Self::UpdateConnectionParamsComplete(_) => "UpdateConnectionParamsComplete",
//                 Self::SetPacketLengthComplete(_) => "SetPacketLengthComplete",
//                 Self::SetLocalPrivacy(_) => "SetLocalPrivacy",
//                 Self::RemoveDeviceBondComplete(_) => "RemoveDeviceBondComplete",
//                 Self::ClearDeviceBondComplete(_) => "ClearDeviceBondComplete",
//                 Self::GetDeviceBondComplete(_) => "GetDeviceBondComplete",
//                 Self::ReadRssiComplete(_) => "ReadRssiComplete",
//                 Self::UpdateWhitelistComplete(_) => "UpdateWhitelistComplete",
//                 Self::UpdateDuplicateListComplete(_) => "UpdateDuplicateListComplete",
//                 Self::SetChannelsComplete(_) => "SetChannelsComplete",
//             }
//         )
//     }
// }

pub struct EspGap<'d, M, T>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BleEnabled,
{
    _driver: T,
    _p: PhantomData<&'d ()>,
    _m: PhantomData<M>,
}

impl<'d, M, T> EspGap<'d, M, T>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BleEnabled,
{
    pub fn new<F>(driver: T, events_cb: F) -> Result<Self, EspError>
    where
        F: Fn(GapEvent) + Send + 'static,
    {
        CALLBACK.set(events_cb)?;

        esp!(unsafe { esp_bt_gap_register_callback(Some(Self::event_handler)) })?;

        Ok(Self {
            _driver: driver,
            _p: PhantomData,
            _m: PhantomData,
        })
    }

    pub fn set_scan_mode(
        &mut self,
        connectable: bool,
        discovery_mode: DiscoveryMode,
    ) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_set_scan_mode(connectable as _, discovery_mode as _) })
    }

    pub fn start_discovery(
        &mut self,
        inq_mode: InqMode,
        inq_duration: u8,
        num_rsps: usize,
    ) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_start_discovery(inq_mode as _, inq_duration, num_rsps as _) })
    }

    pub fn stop_discovery(&mut self) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_cancel_discovery() })
    }

    pub fn request_remote_services(&mut self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_get_remote_services(bd_addr as *const _ as *mut _) })
    }

    pub fn resolve_eir_data(&mut self, eir: &[u8], eir_type: EirType) -> Result<&[u8], EspError> {
        let mut len = 0;
        let addr = unsafe {
            esp_bt_gap_resolve_eir_data(eir.as_ptr() as *mut _, eir_type.into(), &mut len)
        };

        Ok(unsafe { core::slice::from_raw_parts(addr, len as _) })
    }

    pub fn set_eir_data_conf(&mut self, eir_data: &EirData) -> Result<(), EspError> {
        let data = eir_data.into();
        esp!(unsafe { esp_bt_gap_config_eir_data(&data as *const esp_bt_eir_data_t as *mut _) })
    }

    pub fn get_cod(&mut self) -> Result<Cod, EspError> {
        let mut cod = Default::default();
        esp!(unsafe { esp_bt_gap_get_cod(&mut cod) })?;

        Ok(cod.into())
    }

    pub fn set_cod(&mut self, cod: Cod, mode: EnumSet<CodMode>) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_set_cod(cod.into(), mode.as_repr()) })
    }

    pub fn request_rssi_delta(&mut self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_read_rssi_delta(bd_addr as *const _ as *mut _) })
    }

    pub fn get_bond_services<'a>(
        &mut self,
        buf: &'a mut [BdAddr],
    ) -> Result<(&'a [BdAddr], usize), EspError> {
        let mut dev_num = buf.len() as _;

        esp!(unsafe { esp_bt_gap_get_bond_device_list(&mut dev_num, buf.as_ptr() as *mut _) })?;

        Ok((&buf[..min(dev_num as _, buf.len())], dev_num as _))
    }

    pub fn remove_bond_service(&mut self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_remove_bond_device(bd_addr as *const _ as *mut _) })
    }

    pub fn set_pin(&mut self, pin: &[u8]) -> Result<(), EspError> {
        esp!(unsafe {
            esp_bt_gap_set_pin(
                esp_bt_pin_type_t_ESP_BT_PIN_TYPE_FIXED,
                pin.len() as _,
                pin.as_ptr() as *mut _,
            )
        })
    }

    pub fn request_variable_pin(&mut self) -> Result<(), EspError> {
        esp!(unsafe {
            esp_bt_gap_set_pin(
                esp_bt_pin_type_t_ESP_BT_PIN_TYPE_VARIABLE,
                0,
                core::ptr::null_mut(),
            )
        })
    }

    pub fn reply_variable_pin(
        &mut self,
        bd_addr: &BdAddr,
        pin: Option<&[u8]>,
    ) -> Result<(), EspError> {
        esp!(unsafe {
            esp_bt_gap_pin_reply(
                bd_addr as *const _ as *mut _,
                pin.is_some(),
                pin.map_or(0, |pin| pin.len() as _),
                pin.map_or(core::ptr::null_mut(), |pin| pin.as_ptr() as *mut _),
            )
        })
    }

    pub fn reply_passkey(
        &mut self,
        bd_addr: &BdAddr,
        passkey: Option<u32>,
    ) -> Result<(), EspError> {
        esp!(unsafe {
            esp_bt_gap_ssp_passkey_reply(
                bd_addr as *const _ as *mut _,
                passkey.is_some(),
                passkey.unwrap_or(0),
            )
        })
    }

    pub fn reply_confirm(&mut self, bd_addr: &BdAddr, confirm: bool) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_ssp_confirm_reply(bd_addr as *const _ as *mut _, confirm) })
    }

    pub fn set_afh_channels_conf(&mut self, channels: &[u8; 10]) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_set_afh_channels(channels as *const _ as *mut _) })
    }

    pub fn request_remote_name(&mut self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_read_remote_name(bd_addr as *const _ as *mut _) })
    }

    pub fn set_qos_conf(&mut self, bd_addr: &BdAddr, poll: u32) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_set_qos(bd_addr as *const _ as *mut _, poll) })
    }

    unsafe extern "C" fn event_handler(
        event: esp_bt_gap_cb_event_t,
        param: *mut esp_bt_gap_cb_param_t,
    ) {
        let param = unsafe { param.as_ref() }.unwrap();
        let event = GapEvent::from((event, param));

        //debug!("Got GAP event {{ {:#?} }}", event);

        CALLBACK.call(event);
    }
}

impl<'d, M, T> Drop for EspGap<'d, M, T>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BleEnabled,
{
    fn drop(&mut self) {
        esp!(unsafe { esp_bt_gap_register_callback(None) }).unwrap();

        CALLBACK.clear().unwrap();
    }
}

static CALLBACK: BtCallback<GapEvent, ()> = BtCallback::new(());
