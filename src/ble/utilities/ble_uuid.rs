use esp_idf_sys::{
    esp_bt_uuid_t, esp_gatt_id_t, ESP_UUID_LEN_128, ESP_UUID_LEN_16, ESP_UUID_LEN_32,
};

/// A Bluetooth UUID.
#[derive(Copy, Clone)]
pub enum BleUuid {
    /// A 16-bit UUID.
    Uuid16(u16),
    /// A 32-bit UUID.
    Uuid32(u32),
    /// A 128-bit UUID.
    Uuid128([u8; 16]),
}

impl BleUuid {
    /// Creates a new [`BleUuid`] from a 16-bit integer.
    #[must_use]
    pub const fn from_uuid16(uuid: u16) -> Self {
        Self::Uuid16(uuid)
    }

    /// Creates a new [`BleUuid`] from a 32-bit integer.
    #[must_use]
    pub const fn from_uuid32(uuid: u32) -> Self {
        Self::Uuid32(uuid)
    }

    /// Creates a new [`BleUuid`] from a 16 byte array.
    #[must_use]
    pub const fn from_uuid128(uuid: [u8; 16]) -> Self {
        Self::Uuid128(uuid)
    }

    /// Creates a new [`BleUuid`] from a formatted string.
    ///
    /// # Panics
    ///
    /// Panics if the string contains invalid characters.
    pub fn from_uuid128_string<S: AsRef<str>>(uuid: S) -> Self {
        // Accepts the following formats:
        // "00000000-0000-0000-0000-000000000000"
        // "00000000000000000000000000000000"

        let uuid = uuid.as_ref();

        let mut uuid_bytes = [0u8; 16];
        // Remove the dashes.
        let uuid = uuid.replace('-', "");

        for (i, byte) in uuid.as_bytes().chunks(2).enumerate() {
            uuid_bytes[i] = u8::from_str_radix(std::str::from_utf8(byte).unwrap(), 16).unwrap();
        }

        uuid_bytes.reverse();
        Self::Uuid128(uuid_bytes)
    }

    #[must_use]
    pub(crate) fn as_uuid128_array(&self) -> [u8; 16] {
        let base_ble_uuid = [
            0xfb, 0x34, 0x9b, 0x5f, 0x80, 0x00, 0x00, 0x80, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];

        match self {
            Self::Uuid16(uuid) => {
                let mut uuid128 = base_ble_uuid;

                let mut uuid_as_bytes: [u8; 2] = uuid.to_be_bytes();
                uuid_as_bytes.reverse();

                uuid128[12..=13].copy_from_slice(&uuid_as_bytes[..]);
                uuid128
            }
            Self::Uuid32(uuid) => {
                let mut uuid128 = base_ble_uuid;

                let mut uuid_as_bytes: [u8; 4] = uuid.to_be_bytes();
                uuid_as_bytes.reverse();

                uuid128[12..=15].copy_from_slice(&uuid_as_bytes[..]);
                uuid128
            }
            Self::Uuid128(uuid) => *uuid,
        }
    }
}

impl PartialEq for BleUuid {
    fn eq(&self, other: &Self) -> bool {
        self.as_uuid128_array() == other.as_uuid128_array()
    }
}

impl From<BleUuid> for esp_gatt_id_t {
    fn from(val: BleUuid) -> Self {
        Self {
            uuid: val.into(),
            inst_id: 0x00,
        }
    }
}

impl From<BleUuid> for esp_bt_uuid_t {
    #[allow(clippy::cast_possible_truncation)]
    fn from(val: BleUuid) -> Self {
        let mut result: Self = Self::default();

        match val {
            BleUuid::Uuid16(uuid) => {
                result.len = ESP_UUID_LEN_16 as u16;
                result.uuid.uuid16 = uuid;
            }
            BleUuid::Uuid32(uuid) => {
                result.len = ESP_UUID_LEN_32 as u16;
                result.uuid.uuid32 = uuid;
            }
            BleUuid::Uuid128(uuid) => {
                result.len = ESP_UUID_LEN_128 as u16;
                result.uuid.uuid128 = uuid;
            }
        }

        result
    }
}

impl From<esp_bt_uuid_t> for BleUuid {
    fn from(uuid: esp_bt_uuid_t) -> Self {
        unsafe {
            match uuid.len {
                2 => Self::Uuid16(uuid.uuid.uuid16),
                4 => Self::Uuid32(uuid.uuid.uuid32),
                16 => Self::Uuid128(uuid.uuid.uuid128),
                // Never happens
                _ => unreachable!("Invalid UUID length"),
            }
        }
    }
}

impl From<esp_gatt_id_t> for BleUuid {
    fn from(uuid: esp_gatt_id_t) -> Self {
        Self::from(uuid.uuid)
    }
}

impl std::fmt::Display for BleUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uuid16(uuid) => write!(f, "0x{uuid:04x}"),
            Self::Uuid32(uuid) => write!(f, "0x{uuid:08x}"),
            Self::Uuid128(uuid) => {
                let mut uuid = *uuid;
                uuid.reverse();

                let mut uuid_str = String::new();

                for byte in &uuid {
                    uuid_str.push_str(&format!("{byte:02x}"));
                }
                uuid_str.insert(8, '-');
                uuid_str.insert(13, '-');
                uuid_str.insert(18, '-');
                uuid_str.insert(23, '-');

                write!(f, "{uuid_str}")
            }
        }
    }
}

impl std::fmt::Debug for BleUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}
