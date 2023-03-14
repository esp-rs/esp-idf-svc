use esp_idf_sys::*;
use log::warn;

/// Represents the properties of a [`Characteristic`].
///
/// These are the properties that the device announces to the client.
///
/// # Notes
///
/// Keep in mind that you *must* set the [`read`] and [`write`] properties in the same way as
/// the ones in [`AttributePermissions`]. Otherwise, the client might issue a read or write command
/// to a [`Characteristic`] that doesn't allow it.
///
/// [`AttributePermissions`]: crate::utilities::AttributePermissions
/// [`Characteristic`]: crate::gatt_server::characteristic::Characteristic
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CharacteristicProperties {
    broadcast: bool,
    pub(crate) read: bool,
    pub(crate) write_without_response: bool,
    pub(crate) write: bool,
    pub(crate) notify: bool,
    pub(crate) indicate: bool,
    authenticated_signed_writes: bool,
    extended_properties: bool,
}

impl CharacteristicProperties {
    /// Creates a new [`CharacteristicProperties`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the "broadcast" property.
    #[must_use]
    pub const fn broadcast(mut self) -> Self {
        self.broadcast = true;
        self
    }

    /// Sets the "read" property.
    #[must_use]
    pub const fn read(mut self) -> Self {
        self.read = true;
        self
    }

    /// Sets the "write without response" property.
    #[must_use]
    pub const fn write_without_response(mut self) -> Self {
        self.write_without_response = true;
        self
    }

    /// Sets the "write" property.
    #[must_use]
    pub const fn write(mut self) -> Self {
        self.write = true;
        self
    }

    /// Sets the "notify" property.
    #[must_use]
    pub fn notify(mut self) -> Self {
        if self.indicate {
            warn!("Cannot set notify and indicate at the same time. Ignoring notify.");
            return self;
        }

        self.notify = true;
        self
    }

    /// Sets the "indicate" property.
    #[must_use]
    pub fn indicate(mut self) -> Self {
        if self.notify {
            warn!("Cannot set notify and indicate at the same time. Ignoring indicate.");
            return self;
        }

        self.indicate = true;
        self
    }

    /// Sets the "authenticated signed writes" property.
    #[must_use]
    pub const fn authenticated_signed_writes(mut self) -> Self {
        self.authenticated_signed_writes = true;
        self
    }

    /// Sets the "extended properties" property.
    #[must_use]
    pub const fn extended_properties(mut self) -> Self {
        self.extended_properties = true;
        self
    }
}

impl From<CharacteristicProperties> for esp_gatt_char_prop_t {
    #[allow(clippy::cast_possible_truncation)]
    fn from(properties: CharacteristicProperties) -> Self {
        let mut result = 0;
        if properties.broadcast {
            result |= ESP_GATT_CHAR_PROP_BIT_BROADCAST;
        }
        if properties.read {
            result |= ESP_GATT_CHAR_PROP_BIT_READ;
        }
        if properties.write_without_response {
            result |= ESP_GATT_CHAR_PROP_BIT_WRITE_NR;
        }
        if properties.write {
            result |= ESP_GATT_CHAR_PROP_BIT_WRITE;
        }
        if properties.notify {
            result |= ESP_GATT_CHAR_PROP_BIT_NOTIFY;
        }
        if properties.indicate {
            result |= ESP_GATT_CHAR_PROP_BIT_INDICATE;
        }
        if properties.authenticated_signed_writes {
            result |= ESP_GATT_CHAR_PROP_BIT_AUTH;
        }
        if properties.extended_properties {
            result |= ESP_GATT_CHAR_PROP_BIT_EXT_PROP;
        }
        result as Self
    }
}
