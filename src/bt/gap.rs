use core::cmp::min;
use core::ops::BitOr;
use core::{
    borrow::Borrow,
    fmt::{self, Debug},
    marker::PhantomData,
};

use enumset::{EnumSet, EnumSetType};

use esp_idf_sys::*;

use log::{debug, info};

use crate::bt::BtCallback;
use crate::{
    bt::{BleEnabled, BtDriver, BtUuid},
    private::cstr::to_cstring_arg,
};

pub type BdAddr = esp_bd_addr_t;

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

pub type EirType = esp_bt_eir_type_t;

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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Cod {
    pub minor: u8,
    pub major: u8,
    pub service: u16,
}

impl From<Cod> for esp_bt_cod_t {
    fn from(data: Cod) -> Self {
        let mut cod: esp_bt_cod_t = Default::default();

        cod.set_minor(data.minor as _);
        cod.set_major(data.major as _);
        cod.set_service(data.service as _);

        cod
    }
}

impl From<esp_bt_cod_t> for Cod {
    fn from(data: esp_bt_cod_t) -> Self {
        Self {
            minor: data.minor() as _,
            major: data.major() as _,
            service: data.service() as _,
        }
    }
}

#[derive(Clone)]
pub enum GapEvent {
    DeviceDiscoveryStarted,
    DeviceDiscoveryStopped,
    DeviceDiscovered,
    RemoteServiceDiscovered,
    RemoteServiceDetails,
    AuthenticationCompleted,
    PairingPinRequest,
    PairingUserConfirmationRequest,
    PairingPasskeyRequest,
    PairingPasskey,
    Rssi,
    EirDataConfigured,
    AfhChannelsConfigured,
    RemoteName,
    ModeChange,
    BondedServiceRemoved,
    AclConnected,
    AclDisconnected,
}

#[allow(non_upper_case_globals)]
impl From<(esp_bt_gap_cb_event_t, &esp_bt_gap_cb_param_t)> for GapEvent {
    fn from(value: (esp_bt_gap_cb_event_t, &esp_bt_gap_cb_param_t)) -> Self {
        let (evt, param) = value;

        unsafe {
            match evt {
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
                    panic!("Unhandled event {:?}", evt)
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

    pub fn get_remote_services(&mut self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_bt_gap_get_remote_services(bd_addr as *const _ as *mut _) })
    }

    pub fn resolve_eir_data(&mut self, eir: &[u8], eir_type: EirType) -> Result<&[u8], EspError> {
        let mut len = 0;
        let addr =
            unsafe { esp_bt_gap_resolve_eir_data(eir.as_ptr() as *mut _, eir_type as _, &mut len) };

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

    pub fn schedule_variable_pin(&mut self) -> Result<(), EspError> {
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

    pub fn set_afh_coannels_conf(&mut self, channels: &[u8; 10]) -> Result<(), EspError> {
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

static CALLBACK: BtCallback<GapEvent> = BtCallback::new();
