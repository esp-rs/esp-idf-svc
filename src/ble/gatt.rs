//! NimBLE GATT: shared types and helpers.

use core::ffi::c_int;

#[cfg(esp_idf_bt_nimble_gatt_server)]
use enumset::{EnumSet, EnumSetType};

use crate::sys::*;

use super::{BleError, ConnHandle};

#[cfg(esp_idf_bt_nimble_gatt_client)]
pub mod client;
#[cfg(esp_idf_bt_nimble_gatt_server)]
pub mod server;

/// A GATT attribute handle (e.g. a characteristic's value handle).
pub type AttrHandle = u16;

/// Set the preferred ATT MTU. Safe to call before or after the host starts.
pub fn set_preferred_mtu(mtu: u16) -> Result<(), BleError> {
    BleError::from_raw(unsafe { ble_att_set_preferred_mtu(mtu) })
}

/// The negotiated ATT MTU for a connection
pub fn att_mtu(conn_handle: ConnHandle) -> Result<u16, BleError> {
    match unsafe { ble_att_mtu(conn_handle) } {
        0 => Err(BleError::new(BLE_HS_ENOTCONN as c_int)),
        mtu => Ok(mtu),
    }
}

/// A GATT characteristic operation flag (`BLE_GATT_CHR_F_*`).
#[cfg(esp_idf_bt_nimble_gatt_server)]
#[derive(Debug, EnumSetType)]
pub enum BleGattCharFlag {
    Read,
    Write,
    Notify,
    Indicate,
}

#[cfg(esp_idf_bt_nimble_gatt_server)]
impl BleGattCharFlag {
    /// The raw NimBLE flag bit (`BLE_GATT_CHR_F_*`). `const`, so it can be used to build a static
    /// service table (see the [`gatt_services!`](crate::gatt_services) macro).
    pub const fn repr(self) -> ble_gatt_chr_flags {
        match self {
            Self::Read => BLE_GATT_CHR_F_READ,
            Self::Write => BLE_GATT_CHR_F_WRITE,
            Self::Notify => BLE_GATT_CHR_F_NOTIFY,
            Self::Indicate => BLE_GATT_CHR_F_INDICATE,
        }
    }
}

#[cfg(esp_idf_bt_nimble_gatt_server)]
impl From<BleGattCharFlag> for ble_gatt_chr_flags {
    fn from(flag: BleGattCharFlag) -> Self {
        flag.repr()
    }
}

#[cfg(esp_idf_bt_nimble_gatt_server)]
pub(crate) fn flags_to_repr(flags: EnumSet<BleGattCharFlag>) -> ble_gatt_chr_flags {
    flags
        .iter()
        .fold(0, |acc, flag| acc | ble_gatt_chr_flags::from(flag))
}
