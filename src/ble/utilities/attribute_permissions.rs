use esp_idf_sys::*;

/// Represents an attribute's access permissions.
///
/// This struct is used to set the permissions of a [`Characteristic`] or a [`Descriptor`].
/// It can represent read and write permissions, and encryption requirements.
#[derive(Debug, Clone, Copy, Default)]
pub struct AttributePermissions {
    pub(crate) read_access: bool,
    pub(crate) write_access: bool,
    pub(crate) encryption_required: bool,
}

impl AttributePermissions {
    /// Creates a new [`AttributePermissions`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the read access of the [`AttributePermissions`].
    #[must_use]
    pub const fn read(mut self) -> Self {
        self.read_access = true;
        self
    }

    /// Sets the write access of the [`AttributePermissions`].
    #[must_use]
    pub const fn write(mut self) -> Self {
        self.write_access = true;
        self
    }

    /// Sets the encryption requirement of the [`AttributePermissions`].
    #[must_use]
    pub const fn encrypted(mut self) -> Self {
        self.encryption_required = true;
        self
    }
}

impl From<AttributePermissions> for esp_gatt_perm_t {
    #[allow(clippy::cast_possible_truncation)]
    fn from(permissions: AttributePermissions) -> Self {
        let result = match (
            permissions.read_access,
            permissions.write_access,
            permissions.encryption_required,
        ) {
            // TODO: Implement all the supported modes.
            (false, false, _) => 0,
            (true, false, false) => ESP_GATT_PERM_READ,
            (false, true, false) => ESP_GATT_PERM_WRITE,
            (true, true, false) => ESP_GATT_PERM_READ | ESP_GATT_PERM_WRITE,
            (true, false, true) => ESP_GATT_PERM_READ_ENCRYPTED,
            (false, true, true) => ESP_GATT_PERM_WRITE_ENCRYPTED,
            (true, true, true) => ESP_GATT_PERM_READ_ENCRYPTED | ESP_GATT_PERM_WRITE_ENCRYPTED,
        };

        result as Self
    }
}
