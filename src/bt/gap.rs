use core::cmp::min;
use core::convert::TryInto;
use core::ffi;
use core::fmt::{self, Debug};
use core::sync::atomic::{AtomicBool, Ordering};
use core::{borrow::Borrow, marker::PhantomData};

use enumset::{EnumSet, EnumSetType};

use esp_idf_sys::*;

use log::{debug, info};

use num_enum::TryFromPrimitive;

use super::{BdAddr, BtCallback, BtClassicEnabled, BtDriver, BtUuid};

#[cfg(esp_idf_bt_ssp_enabled)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum IOCapabilities {
    DisplayOnly = ESP_BT_IO_CAP_OUT as _,
    DisplayInput = ESP_BT_IO_CAP_IO as _,
    InputOnly = ESP_BT_IO_CAP_IN as _,
    None = ESP_BT_IO_CAP_NONE as _,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, TryFromPrimitive)]
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

#[derive(Debug)]
#[repr(transparent)]
pub struct Eir<'a>(*const ffi::c_void, PhantomData<&'a ()>);

impl<'a> Eir<'a> {
    pub fn flags(&self) -> Option<EnumSet<EirFlags>> {
        todo!()
    }

    pub fn uuid(&self) -> Option<&BtUuid> {
        todo!()
    }

    // pub fn uui16_incomplete(&self) -> Option<&[u8]> {
    //     todo!()
    // }

    // pub fn uuid32(&self) -> Option<&[u8]> {
    //     todo!()
    // }

    // pub fn uui32_incomplete(&self) -> Option<&[u8]> {
    //     todo!()
    // }

    // pub fn uuid128(&self) -> Option<&[u8]> {
    //     todo!()
    // }

    // pub fn uui128_incomplete(&self) -> Option<&[u8]> {
    //     todo!()
    // }

    pub fn short_local_name(&self) -> Option<&str> {
        todo!()
    }

    pub fn local_name(&self) -> Option<&str> {
        todo!()
    }

    pub fn url(&self) -> Option<&str> {
        todo!()
    }

    pub fn manufacturer_data(&self) -> Option<&[u8]> {
        todo!()
    }

    // pub fn resolve_eir_data(&self, eir: &[u8], eir_type: EirType) -> Result<&[u8], EspError> {
    //     let mut len = 0;
    //     let addr = unsafe {
    //         esp_bt_gap_resolve_eir_data(eir.as_ptr() as *mut _, eir_type.into(), &mut len)
    //     };

    //     Ok(unsafe { core::slice::from_raw_parts(addr, len as _) })
    // }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum EirType {
    Flags = ESP_BT_EIR_TYPE_FLAGS as _,
    Uuid16Incomplete = ESP_BT_EIR_TYPE_INCMPL_16BITS_UUID as _,
    Uuid16 = ESP_BT_EIR_TYPE_CMPL_16BITS_UUID as _,
    Uuid32Incomplete = ESP_BT_EIR_TYPE_INCMPL_32BITS_UUID as _,
    Uuid32 = ESP_BT_EIR_TYPE_CMPL_32BITS_UUID as _,
    Uuid128Incomplete = ESP_BT_EIR_TYPE_INCMPL_128BITS_UUID as _,
    Uuid128 = ESP_BT_EIR_TYPE_CMPL_128BITS_UUID as _,
    ShortLocalName = ESP_BT_EIR_TYPE_SHORT_LOCAL_NAME as _,
    LocalName = ESP_BT_EIR_TYPE_CMPL_LOCAL_NAME as _,
    TxPowerLevel = ESP_BT_EIR_TYPE_TX_POWER_LEVEL as _,
    Url = ESP_BT_EIR_TYPE_URL as _,
    ManufacturerData = ESP_BT_EIR_TYPE_MANU_SPECIFIC as _,
}

#[derive(Debug, EnumSetType)]
#[enumset(repr = "u8")]
#[repr(u8)]
pub enum EirFlags {
    LimitDisc = 0,        //ESP_BT_EIR_FLAG_LIMIT_DISC as _,
    GenDisc = 1,          //ESP_BT_EIR_FLAG_GEN_DISC as _,
    BredrNotSpt = 2,      //ESP_BT_EIR_FLAG_BREDR_NOT_SPT as _,
    DmtControllerSpt = 3, //ESP_BT_EIR_FLAG_DMT_CONTROLLER_SPT as _,
    DmtHostSpt = 4,       //ESP_BT_EIR_FLAG_DMT_HOST_SPT as _,
}

#[derive(Debug, Clone)]
pub struct EirData<'a> {
    pub fec_required: bool,          // FEC is required or not, true by default
    pub include_txpower: bool,       // EIR data include TX power, false by default
    pub include_uuid: bool,          // EIR data include UUID, false by default
    pub flags: EnumSet<EirFlags>, // EIR flags, see ESP_BT_EIR_FLAG for details, EIR will not include flag if it is 0, 0 by default
    pub manufacturer_data: &'a [u8], // Manufacturer data point
    pub url: &'a str,             // URL point
}

impl<'a> From<&EirData<'a>> for esp_bt_eir_data_t {
    fn from(data: &EirData<'a>) -> Self {
        Self {
            fec_required: data.fec_required,
            include_txpower: data.include_txpower,
            include_uuid: data.include_uuid,
            flag: data.flags.as_repr(),
            p_manufacturer_data: data.manufacturer_data.as_ptr() as *mut _,
            manufacturer_len: data.manufacturer_data.len() as _,
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
            manufacturer_data: unsafe {
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

#[derive(Debug, Copy, Clone, Eq, PartialEq, TryFromPrimitive)]
#[repr(u32)]
enum CodService {
    Miscellaneous = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_MISC,
    Computer = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_COMPUTER,
    Phone = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_PHONE,
    Lan = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_LAN_NAP,
    AudioVideo = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_AV,
    Peripheral = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_PERIPHERAL,
    Imaging = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_IMAGING,
    Wearable = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_WEARABLE,
    Toy = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_TOY,
    Health = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_HEALTH,
    Other = esp_bt_cod_major_dev_t_ESP_BT_COD_MAJOR_DEV_UNCATEGORIZED,
}

enum CodMajorDeviceClass {}

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
    Eir(Eir<'a>),
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
                    DeviceProp::Eir(Eir(self.data.val, PhantomData))
                }
                _ => unreachable!(),
            }
        }
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

pub struct EventRawData<'a>(pub &'a esp_bt_gap_cb_param_t);

impl<'a> Debug for EventRawData<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("EventRawData").finish()
    }
}

#[derive(Debug)]
pub enum GapEvent<'a> {
    DeviceDiscoveryStarted,
    DeviceDiscoveryStopped,
    DeviceDiscovered {
        bd_addr: BdAddr,
        props: &'a [PropData<'a>],
    },
    RemoteServiceDiscovered {
        bd_addr: BdAddr,
        status: BtStatus,
        services: &'a [BtUuid],
    },
    RemoteServiceDetails {
        bd_addr: BdAddr,
        status: BtStatus,
    },
    AuthenticationCompleted {
        bd_addr: BdAddr,
        status: BtStatus,
        device_name: &'a str,
    },
    PairingPinRequest {
        bd_addr: BdAddr,
        min_16_digit: bool,
    },
    PairingUserConfirmationRequest {
        bd_addr: BdAddr,
        number: u32,
    },
    SspPasskey {
        bd_addr: BdAddr,
        passkey: u32,
    },
    SspPasskeyRequest {
        bd_addr: BdAddr,
    },
    Rssi {
        bd_addr: BdAddr,
        status: BtStatus,
        rssi: i8,
    },
    EirDataConfigured {
        status: BtStatus,
        eir_types: &'a [EirType],
    },
    AfhChannelsConfigured(BtStatus),
    RemoteName {
        bd_addr: BdAddr,
        status: BtStatus,
        name: &'a str,
    },
    ModeChange {
        bd_addr: BdAddr,
        mode: u8,
    },
    BondedServiceRemoved {
        bd_addr: BdAddr,
        status: BtStatus,
    },
    AclConnected {
        bd_addr: BdAddr,
        status: BtStatus,
        handle: u16,
    },
    AclDisconnected {
        bd_addr: BdAddr,
        status: BtStatus,
        handle: u16,
    },
    QosComplete {
        bd_addr: BdAddr,
        status: BtStatus,
        t_poll: u32,
    },
    Other {
        raw_event: esp_bt_gap_cb_event_t,
        raw_data: EventRawData<'a>,
    },
}

#[allow(non_upper_case_globals)]
impl<'a> From<(esp_bt_gap_cb_event_t, &'a esp_bt_gap_cb_param_t)> for GapEvent<'a> {
    fn from(value: (esp_bt_gap_cb_event_t, &'a esp_bt_gap_cb_param_t)) -> Self {
        let (event, param) = value;

        unsafe {
            match event {
                esp_bt_gap_cb_event_t_ESP_BT_GAP_DISC_RES_EVT => Self::DeviceDiscovered {
                    bd_addr: param.disc_res.bda.into(),
                    props: core::slice::from_raw_parts(
                        param.disc_res.prop as *mut PropData as *const _,
                        param.disc_res.num_prop as _,
                    ),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_DISC_STATE_CHANGED_EVT => {
                    if param.disc_st_chg.state
                        == esp_bt_gap_discovery_state_t_ESP_BT_GAP_DISCOVERY_STOPPED
                    {
                        Self::DeviceDiscoveryStopped
                    } else {
                        Self::DeviceDiscoveryStarted
                    }
                }
                esp_bt_gap_cb_event_t_ESP_BT_GAP_RMT_SRVCS_EVT => Self::RemoteServiceDiscovered {
                    bd_addr: param.rmt_srvcs.bda.into(),
                    status: param.rmt_srvcs.stat.try_into().unwrap(),
                    services: core::slice::from_raw_parts(
                        param.rmt_srvcs.uuid_list as *mut BtUuid as *const _,
                        param.rmt_srvcs.num_uuids as _,
                    ),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_RMT_SRVC_REC_EVT => Self::RemoteServiceDetails {
                    bd_addr: param.rmt_srvcs.bda.into(),
                    status: param.rmt_srvcs.stat.try_into().unwrap(),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_AUTH_CMPL_EVT => Self::AuthenticationCompleted {
                    bd_addr: param.auth_cmpl.bda.into(),
                    status: param.auth_cmpl.stat.try_into().unwrap(),
                    device_name: core::str::from_utf8_unchecked(
                        &param.auth_cmpl.device_name
                            [..strlen(&param.auth_cmpl.device_name as *const _ as *const _) as _],
                    ),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_PIN_REQ_EVT => Self::PairingPinRequest {
                    bd_addr: param.pin_req.bda.into(),
                    min_16_digit: param.pin_req.min_16_digit,
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_CFM_REQ_EVT => {
                    Self::PairingUserConfirmationRequest {
                        bd_addr: param.cfm_req.bda.into(),
                        number: param.cfm_req.num_val,
                    }
                }
                esp_bt_gap_cb_event_t_ESP_BT_GAP_KEY_NOTIF_EVT => Self::SspPasskey {
                    bd_addr: param.key_notif.bda.into(),
                    passkey: param.key_notif.passkey,
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_KEY_REQ_EVT => Self::SspPasskeyRequest {
                    bd_addr: param.key_req.bda.into(),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_READ_RSSI_DELTA_EVT => Self::Rssi {
                    bd_addr: param.read_rssi_delta.bda.into(),
                    status: param.read_rssi_delta.stat.try_into().unwrap(),
                    rssi: param.read_rssi_delta.rssi_delta,
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_CONFIG_EIR_DATA_EVT => Self::EirDataConfigured {
                    status: param.config_eir_data.stat.try_into().unwrap(),
                    eir_types: core::mem::transmute(
                        &param.config_eir_data.eir_type[..param.config_eir_data.eir_type_num as _],
                    ),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_SET_AFH_CHANNELS_EVT => {
                    Self::AfhChannelsConfigured(param.set_afh_channels.stat.try_into().unwrap())
                }
                esp_bt_gap_cb_event_t_ESP_BT_GAP_READ_REMOTE_NAME_EVT => Self::RemoteName {
                    bd_addr: param.read_rmt_name.bda.into(),
                    status: param.read_rmt_name.stat.try_into().unwrap(),
                    name: core::str::from_utf8_unchecked(
                        &param.read_rmt_name.rmt_name
                            [..strlen(param.read_rmt_name.rmt_name.as_ptr() as *const _) as _],
                    ),
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_MODE_CHG_EVT => Self::ModeChange {
                    bd_addr: param.mode_chg.bda.into(),
                    mode: param.mode_chg.mode,
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_REMOVE_BOND_DEV_COMPLETE_EVT => {
                    Self::BondedServiceRemoved {
                        bd_addr: param.remove_bond_dev_cmpl.bda.into(),
                        status: param.remove_bond_dev_cmpl.status.try_into().unwrap(),
                    }
                }
                esp_bt_gap_cb_event_t_ESP_BT_GAP_QOS_CMPL_EVT => Self::QosComplete {
                    bd_addr: param.qos_cmpl.bda.into(),
                    status: param.qos_cmpl.stat.try_into().unwrap(),
                    t_poll: param.qos_cmpl.t_poll,
                },
                esp_bt_gap_cb_event_t_ESP_BT_GAP_ACL_CONN_CMPL_STAT_EVT => Self::AclConnected {
                    bd_addr: param.acl_conn_cmpl_stat.bda.into(),
                    status: param.acl_conn_cmpl_stat.stat.try_into().unwrap(),
                    handle: param.acl_conn_cmpl_stat.handle,
                },
                // #[doc = "< ACL disconnection complete status event"]
                esp_bt_gap_cb_event_t_ESP_BT_GAP_ACL_DISCONN_CMPL_STAT_EVT => {
                    Self::AclDisconnected {
                        bd_addr: param.acl_disconn_cmpl_stat.bda.into(),
                        status: param.acl_disconn_cmpl_stat.reason.try_into().unwrap(),
                        handle: param.acl_disconn_cmpl_stat.handle,
                    }
                }
                _ => Self::Other {
                    raw_event: event,
                    raw_data: EventRawData(param),
                },
            }
        }
    }
}

pub struct EspGap<'d, M, T>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
{
    _driver: T,
    initialized: AtomicBool,
    _p: PhantomData<&'d ()>,
    _m: PhantomData<M>,
}

impl<'d, M, T> EspGap<'d, M, T>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
{
    pub const fn new(driver: T) -> Result<Self, EspError> {
        Ok(Self {
            _driver: driver,
            initialized: AtomicBool::new(false),
            _p: PhantomData,
            _m: PhantomData,
        })
    }

    pub fn initialize<F>(&self, events_cb: F) -> Result<(), EspError>
    where
        F: Fn(GapEvent) + Send + 'd,
    {
        CALLBACK.set(events_cb)?;

        esp!(unsafe { esp_bt_gap_register_callback(Some(Self::event_handler)) })?;

        self.initialized.store(true, Ordering::SeqCst);

        Ok(())
    }

    pub fn set_scan_mode(
        &self,
        connectable: bool,
        discovery_mode: DiscoveryMode,
    ) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_set_scan_mode(connectable as _, discovery_mode as _) })
    }

    pub fn start_discovery(
        &self,
        inq_mode: InqMode,
        inq_duration: u8,
        num_rsps: usize,
    ) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_start_discovery(inq_mode as _, inq_duration, num_rsps as _) })
    }

    pub fn stop_discovery(&self) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_cancel_discovery() })
    }

    pub fn request_remote_services(&self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_get_remote_services(bd_addr as *const _ as *mut _) })
    }

    pub fn set_eir_data_conf(&self, eir_data: &EirData) -> Result<(), EspError> {
        let data = eir_data.into();
        esp!(unsafe { esp_bt_gap_config_eir_data(&data as *const esp_bt_eir_data_t as *mut _) })
    }

    pub fn get_cod(&self) -> Result<Cod, EspError> {
        let mut cod = Default::default();
        esp!(unsafe { esp_bt_gap_get_cod(&mut cod) })?;

        Ok(cod.into())
    }

    pub fn set_cod(&self, cod: Cod, mode: EnumSet<CodMode>) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_set_cod(cod.into(), mode.as_repr()) })
    }

    pub fn request_rssi_delta(&self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_read_rssi_delta(bd_addr as *const _ as *mut _) })
    }

    pub fn get_bond_services<'a>(
        &self,
        buf: &'a mut [BdAddr],
    ) -> Result<(&'a [BdAddr], usize), EspError> {
        let mut dev_num = buf.len() as _;

        esp!(unsafe { esp_bt_gap_get_bond_device_list(&mut dev_num, buf.as_ptr() as *mut _) })?;

        Ok((&buf[..min(dev_num as _, buf.len())], dev_num as _))
    }

    pub fn remove_bond_service(&self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_remove_bond_device(bd_addr as *const _ as *mut _) })
    }

    pub fn set_pin(&self, pin: &str) -> Result<(), EspError> {
        esp!(unsafe {
            esp_bt_gap_set_pin(
                esp_bt_pin_type_t_ESP_BT_PIN_TYPE_FIXED,
                pin.len() as _,
                pin.as_ptr() as *mut _,
            )
        })
    }

    pub fn request_variable_pin(&self) -> Result<(), EspError> {
        esp!(unsafe {
            esp_bt_gap_set_pin(
                esp_bt_pin_type_t_ESP_BT_PIN_TYPE_VARIABLE,
                0,
                core::ptr::null_mut(),
            )
        })
    }

    pub fn reply_variable_pin(&self, bd_addr: &BdAddr, pin: Option<&[u8]>) -> Result<(), EspError> {
        esp!(unsafe {
            esp_bt_gap_pin_reply(
                bd_addr as *const _ as *mut _,
                pin.is_some(),
                pin.map_or(0, |pin| pin.len() as _),
                pin.map_or(core::ptr::null_mut(), |pin| pin.as_ptr() as *mut _),
            )
        })
    }

    pub fn reply_passkey(&self, bd_addr: &BdAddr, passkey: Option<u32>) -> Result<(), EspError> {
        esp!(unsafe {
            esp_bt_gap_ssp_passkey_reply(
                bd_addr as *const _ as *mut _,
                passkey.is_some(),
                passkey.unwrap_or(0),
            )
        })
    }

    #[cfg(esp_idf_bt_ssp_enabled)]
    pub fn reply_ssp_confirm(&self, bd_addr: &BdAddr, confirm: bool) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_ssp_confirm_reply(bd_addr as *const _ as *mut _, confirm) })
    }

    pub fn set_afh_channels_conf(&self, channels: &[u8; 10]) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_set_afh_channels(channels as *const _ as *mut _) })
    }

    pub fn request_remote_name(&self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_read_remote_name(bd_addr as *const _ as *mut _) })
    }

    pub fn set_qos_conf(&self, bd_addr: &BdAddr, poll: u32) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_set_qos(bd_addr as *const _ as *mut _, poll) })
    }

    #[cfg(esp_idf_bt_ssp_enabled)]
    pub fn set_ssp_io_cap(&self, io_cap: IOCapabilities) -> Result<(), EspError> {
        let io_cap: esp_bt_io_cap_t = io_cap as _;
        esp!(unsafe {
            esp_bt_gap_set_security_param(
                esp_bt_sp_param_t_ESP_BT_SP_IOCAP_MODE,
                &io_cap as *const _ as *mut ffi::c_void,
                1,
            )
        })
    }

    unsafe extern "C" fn event_handler(
        event: esp_bt_gap_cb_event_t,
        param: *mut esp_bt_gap_cb_param_t,
    ) {
        let param = unsafe { param.as_ref() }.unwrap();
        let event = GapEvent::from((event, param));

        info!("Got event {{ {:#?} }}", event);

        CALLBACK.call(event);
    }
}

impl<'d, M, T> Drop for EspGap<'d, M, T>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
{
    fn drop(&mut self) {
        if self.initialized.load(Ordering::SeqCst) {
            CALLBACK.clear().unwrap();
        }
    }
}

static CALLBACK: BtCallback<GapEvent, ()> = BtCallback::new(());
