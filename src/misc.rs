use esp_idf_sys::*;

pub fn get_default_efuse_mac() -> Result<[u8; 6], EspError> {
    let mut mac = [0; 6];
    unsafe { esp!(esp_efuse_mac_get_default(mac.as_mut_ptr()))? }
    Ok(mac)
}

pub fn restart() {
    unsafe { esp_restart() };
}
