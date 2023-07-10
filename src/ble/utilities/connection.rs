use esp_idf_sys::{
    esp_ble_gatts_cb_param_t_gatts_connect_evt_param,
    esp_ble_gatts_cb_param_t_gatts_disconnect_evt_param,
};

#[derive(Debug, Copy, Clone)]
pub(crate) struct Connection {
    pub(crate) id: u16,
    #[cfg(esp_idf_version_major = "4")]
    pub(crate) is_slave: bool,
    pub(crate) remote_bda: [u8; 6],
}

impl From<esp_ble_gatts_cb_param_t_gatts_connect_evt_param> for Connection {
    fn from(param: esp_ble_gatts_cb_param_t_gatts_connect_evt_param) -> Self {
        Self {
            id: param.conn_id,
            #[cfg(esp_idf_version_major = "4")]
            is_slave: param.link_role == 1,
            remote_bda: param.remote_bda,
        }
    }
}

impl From<esp_ble_gatts_cb_param_t_gatts_disconnect_evt_param> for Connection {
    fn from(param: esp_ble_gatts_cb_param_t_gatts_disconnect_evt_param) -> Self {
        Self {
            id: param.conn_id,
            #[cfg(esp_idf_version_major = "4")]
            is_slave: param.link_role == 1,
            remote_bda: param.remote_bda,
        }
    }
}

#[cfg(esp_idf_version_major = "4")]
impl core::fmt::Display for Connection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ({}, slave: {})",
            self.remote_bda[0],
            self.remote_bda[1],
            self.remote_bda[2],
            self.remote_bda[3],
            self.remote_bda[4],
            self.remote_bda[5],
            self.id,
            self.is_slave,
        )
    }
}

#[cfg(esp_idf_version_major = "5")]
impl core::fmt::Display for Connection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ({})",
            self.remote_bda[0],
            self.remote_bda[1],
            self.remote_bda[2],
            self.remote_bda[3],
            self.remote_bda[4],
            self.remote_bda[5],
            self.id,
        )
    }
}

// TODO: change to the core::hash:Hash
// when new heapless version will be released
// https://github.com/japaric/heapless/blob/644653bf3b831c6bb4963be2de24804acf5e5001/CHANGELOG.md?plain=1#L21-L22
impl hash32::Hash for Connection {
    fn hash<H>(&self, state: &mut H)
    where
        H: hash32::Hasher,
    {
        self.remote_bda.hash(state);
    }
}

impl PartialEq for Connection {
    fn eq(&self, other: &Self) -> bool {
        self.remote_bda == other.remote_bda
    }
}

impl Eq for Connection {}
