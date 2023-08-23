//! Type safe abstraction for esp-tls

use core::{
    ffi::CStr,
    fmt::{Debug, Display},
    time::Duration,
};

use embedded_svc::io;
use esp_idf_sys::{EspError, ESP_ERR_INVALID_SIZE, ESP_ERR_NO_MEM, ESP_FAIL};

use crate::errors::EspIOError;

/// see https://www.ietf.org/rfc/rfc3280.txt ub-common-name-length
const MAX_COMMON_NAME_LENGTH: usize = 64;

pub struct EspTls {
    reader: EspTlsRead,
    writer: EspTlsWrite,
}

impl EspTls {
    pub fn new(host: &str, port: u16, cfg: &Config) -> Result<Self, EspError> {
        let mut rcfg: esp_idf_sys::esp_tls_cfg = unsafe { std::mem::zeroed() };

        if let Some(ca_cert) = cfg.ca_cert {
            rcfg.__bindgen_anon_1.cacert_buf = ca_cert.data().as_ptr();
            rcfg.__bindgen_anon_2.cacert_bytes = ca_cert.data().len() as u32;
        }

        if let Some(client_cert) = cfg.client_cert {
            rcfg.__bindgen_anon_3.clientcert_buf = client_cert.data().as_ptr();
            rcfg.__bindgen_anon_4.clientcert_bytes = client_cert.data().len() as u32;
        }

        if let Some(client_key) = cfg.client_key {
            rcfg.__bindgen_anon_5.clientkey_buf = client_key.data().as_ptr();
            rcfg.__bindgen_anon_6.clientkey_bytes = client_key.data().len() as u32;
        }

        if let Some(ckp) = cfg.client_key_password {
            // TODO: maybe this is not actually required to be NUL-terminated
            rcfg.clientkey_password = ckp.as_ptr() as *const u8;
            rcfg.clientkey_password_len = ckp.to_bytes().len() as u32;
        }

        // allow up to 9 protocols
        let mut alpn_protos: [*const i8; 10];
        let mut alpn_protos_cbuf = [0u8; 99];
        if let Some(protos) = cfg.alpn_protos {
            alpn_protos = cstr_arr_from_str_slice(protos, &mut alpn_protos_cbuf)?;
            rcfg.alpn_protos = alpn_protos.as_mut_ptr();
        }

        rcfg.non_block = cfg.non_block;
        rcfg.use_secure_element = cfg.use_secure_element;
        rcfg.timeout_ms = cfg.timeout_ms as i32;
        rcfg.use_global_ca_store = cfg.use_global_ca_store;

        if let Some(common_name) = cfg.common_name {
            let mut common_name_buf = [0; MAX_COMMON_NAME_LENGTH + 1];
            rcfg.common_name = cstr_from_str(common_name, &mut common_name_buf).as_ptr();
        }

        rcfg.skip_common_name = cfg.skip_common_name;

        let mut raw_kac: esp_idf_sys::tls_keep_alive_cfg;
        if let Some(kac) = &cfg.keep_alive_cfg {
            raw_kac = esp_idf_sys::tls_keep_alive_cfg {
                keep_alive_enable: kac.enable,
                keep_alive_idle: kac.idle.as_secs() as i32,
                keep_alive_interval: kac.interval.as_secs() as i32,
                keep_alive_count: kac.count as i32,
            };
            rcfg.keep_alive_cfg = &mut raw_kac as *mut _;
        }

        let mut raw_psk: esp_idf_sys::psk_key_hint;
        if let Some(psk) = &cfg.psk_hint_key {
            raw_psk = esp_idf_sys::psk_key_hint {
                key: psk.key.as_ptr(),
                key_size: psk.key.len(),
                hint: psk.hint.as_ptr(),
            };
            rcfg.psk_hint_key = &mut raw_psk as *mut _;
        }

        rcfg.is_plain_tcp = cfg.is_plain_tcp;
        rcfg.if_name = std::ptr::null_mut();

        let tls = unsafe { esp_idf_sys::esp_tls_init() };
        if tls == std::ptr::null_mut() {
            return Err(EspError::from_infallible::<ESP_ERR_NO_MEM>());
        }
        let ret = unsafe {
            esp_idf_sys::esp_tls_conn_new_sync(
                host.as_bytes().as_ptr() as *const i8,
                host.len() as i32,
                port as i32,
                &rcfg,
                tls,
            )
        };

        if ret == 1 {
            Ok(EspTls {
                reader: EspTlsRead { raw: tls },
                writer: EspTlsWrite { raw: tls },
            })
        } else {
            unsafe {
                esp_idf_sys::esp_tls_conn_destroy(tls);
            }

            Err(EspError::from_infallible::<ESP_FAIL>())
        }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspIOError> {
        self.reader.read(buf)
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize, EspIOError> {
        self.writer.write(buf)
    }

    pub fn split(&mut self) -> (&mut EspTlsRead, &mut EspTlsWrite) {
        (&mut self.reader, &mut self.writer)
    }
}

impl io::Io for EspTls {
    type Error = EspIOError;
}

impl io::Read for EspTls {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspIOError> {
        self.read(buf)
    }
}

impl io::Write for EspTls {
    fn write(&mut self, buf: &[u8]) -> Result<usize, EspIOError> {
        self.write(buf)
    }

    fn flush(&mut self) -> Result<(), EspIOError> {
        Ok(())
    }
}

pub struct EspTlsRead {
    raw: *mut esp_idf_sys::esp_tls,
}

impl EspTlsRead {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspIOError> {
        // cannot call esp_tls_conn_read bc it's inline
        //let ret = unsafe { esp_idf_sys::esp_tls_conn_read(self.raw, buf.as_mut_ptr(), buf.len()) };
        let esp_tls = unsafe { std::ptr::read_unaligned(self.raw) };
        let read_func = esp_tls.read.unwrap();
        let ret = unsafe { read_func(self.raw, buf.as_mut_ptr() as *mut i8, buf.len()) };
        // ESP docs treat 0 as error, but in Rust it's common to return 0 from `Read::read` to indicate eof
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(EspIOError(EspError::from(ret as i32).unwrap()))
        }
    }
}

impl io::Io for EspTlsRead {
    type Error = EspIOError;
}

impl io::Read for EspTlsRead {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspIOError> {
        self.read(buf)
    }
}

pub struct EspTlsWrite {
    raw: *mut esp_idf_sys::esp_tls,
}

impl EspTlsWrite {
    pub fn write(&mut self, buf: &[u8]) -> Result<usize, EspIOError> {
        // cannot call esp_tls_conn_write bc it's inline
        //let ret = unsafe { esp_idf_sys::esp_tls_conn_write(self.raw, buf.as_ptr(), buf.len()) };
        let esp_tls = unsafe { std::ptr::read_unaligned(self.raw) };
        let write_func = esp_tls.write.unwrap();
        let ret = unsafe { write_func(self.raw, buf.as_ptr() as *const i8, buf.len()) };
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(EspIOError(EspError::from(ret as i32).unwrap()))
        }
    }
}

impl io::Io for EspTlsWrite {
    type Error = EspIOError;
}

impl io::Write for EspTlsWrite {
    fn write(&mut self, buf: &[u8]) -> Result<usize, EspIOError> {
        self.write(buf)
    }

    fn flush(&mut self) -> Result<(), EspIOError> {
        Ok(())
    }
}

impl Drop for EspTls {
    fn drop(&mut self) {
        unsafe {
            esp_idf_sys::esp_tls_conn_destroy(self.reader.raw);
        }
    }
}

pub struct Config<'a> {
    /// up to 9 ALPNs allowed, with avg 10 bytes for each name
    pub alpn_protos: Option<&'a [&'a str]>,
    pub ca_cert: Option<X509<'a>>,
    pub client_cert: Option<X509<'a>>,
    pub client_key: Option<X509<'a>>,
    pub client_key_password: Option<&'a CStr>,
    pub non_block: bool,
    pub use_secure_element: bool,
    pub timeout_ms: u32,
    pub use_global_ca_store: bool,
    pub common_name: Option<&'a str>,
    pub skip_common_name: bool,
    pub keep_alive_cfg: Option<KeepAliveConfig>,
    pub psk_hint_key: Option<PskHintKey<'a>>,
    // TODO crt_bundle_attach
    // TODO ds_data
    pub is_plain_tcp: bool,
    pub if_name: esp_idf_sys::ifreq,
}

impl<'a> Default for Config<'a> {
    fn default() -> Self {
        Self {
            alpn_protos: Default::default(),
            ca_cert: None,
            client_cert: None,
            client_key: None,
            client_key_password: Default::default(),
            non_block: Default::default(),
            use_secure_element: Default::default(),
            timeout_ms: Default::default(),
            use_global_ca_store: Default::default(),
            common_name: Default::default(),
            skip_common_name: Default::default(),
            keep_alive_cfg: Default::default(),
            psk_hint_key: Default::default(),
            is_plain_tcp: Default::default(),
            if_name: Default::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct KeepAliveConfig {
    /// Enable keep-alive timeout
    pub enable: bool,
    /// Keep-alive idle time (second)
    pub idle: Duration,
    /// Keep-alive interval time (second)
    pub interval: Duration,
    /// Keep-alive packet retry send count
    pub count: u32,
}

pub struct PskHintKey<'a> {
    pub key: &'a [u8],
    pub hint: &'a CStr,
}

/// str to cstr, will be truncated if str is larger than buf.len() - 1
///
/// # Panics
///
/// * Panics if buffer is empty.
fn cstr_from_str<'a>(rust_str: &str, buf: &'a mut [u8]) -> &'a CStr {
    assert!(buf.len() > 0);

    let max_str_size = buf.len() - 1; // account for NUL
    let truncated_str = &rust_str[..max_str_size.min(rust_str.len())];
    buf[..truncated_str.len()].copy_from_slice(truncated_str.as_bytes());
    buf[truncated_str.len()] = b'\0';

    CStr::from_bytes_with_nul(&buf[..truncated_str.len() + 1]).unwrap()
}

/// Convert slice of rust strs to NULL-terminated fixed size array of c string pointers
///
/// # Panics
///
/// * Panics if cbuf is empty.
/// * Panics if N is <= 1
fn cstr_arr_from_str_slice<const N: usize>(
    rust_strs: &[&str],
    mut cbuf: &mut [u8],
) -> Result<[*const i8; N], EspError> {
    assert!(N > 1);
    assert!(cbuf.len() > 0);

    // ensure last element stays NULL
    if rust_strs.len() > N - 1 {
        return Err(EspError::from_infallible::<ESP_ERR_INVALID_SIZE>());
    }

    let mut cstrs = [std::ptr::null(); N];

    for (i, s) in rust_strs.into_iter().enumerate() {
        let max_str_size = cbuf.len() - 1; // account for NUL
        if s.len() > max_str_size {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_SIZE>());
        }
        cbuf[..s.len()].copy_from_slice(s.as_bytes());
        cbuf[s.len()] = b'\0';
        let cstr = CStr::from_bytes_with_nul(&cbuf[..s.len() + 1]).unwrap();
        cstrs[i] = cstr.as_ptr();

        cbuf = &mut cbuf[s.len() + 1..];
    }

    Ok(cstrs)
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct X509<'a>(&'a [u8]);

impl<'a> X509<'a> {
    pub fn pem(cstr: &'a CStr) -> Self {
        Self(cstr.to_bytes_with_nul())
    }

    pub const fn pem_until_nul(bytes: &'a [u8]) -> Self {
        // TODO: replace with `CStr::from_bytes_until_nul` when stabilized
        let mut nul_pos = 0;
        while nul_pos < bytes.len() {
            if bytes[nul_pos] == 0 {
                // TODO: replace with `<[u8]>::split_at(nul_pos + 1)` when const stabilized
                let slice = unsafe { core::slice::from_raw_parts(bytes.as_ptr(), nul_pos + 1) };
                return Self(slice);
            }
            nul_pos += 1;
        }
        panic!("PEM certificates should end with a NIL (`\\0`) ASCII character.")
    }

    pub const fn der(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    pub fn data(&self) -> &[u8] {
        self.0
    }

    #[allow(unused)]
    pub(crate) fn as_esp_idf_raw_ptr(&self) -> *const c_char {
        self.data().as_ptr().cast()
    }

    #[allow(unused)]
    pub(crate) fn as_esp_idf_raw_len(&self) -> usize {
        self.data().len()
    }
}

impl<'a> Debug for X509<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        write!(f, "X509(...)")
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CStr;

    use crate::{cstr_arr_from_str_slice, cstr_from_str};

    #[test]
    fn cstr_from_str_happy() {
        let mut same_size = [0u8; 6];
        let hello = cstr_from_str("Hello", &mut same_size);
        assert_eq!(hello.to_bytes(), b"Hello");

        let mut larger = [0u8; 42];
        let hello = cstr_from_str("Hello", &mut larger);
        assert_eq!(hello.to_bytes(), b"Hello");
    }

    #[test]
    fn cstr_from_str_unhappy() {
        let mut smaller = [0u8; 6];
        let hello = cstr_from_str("Hello World", &mut smaller);
        assert_eq!(hello.to_bytes(), b"Hello");
    }

    #[test]
    fn cstr_arr_happy() {
        let mut same_size = [0u8; 13];
        let hello = cstr_arr_from_str_slice::<3>(&["Hello", "World"], &mut same_size).unwrap();
        assert_eq!(unsafe { CStr::from_ptr(hello[0]) }.to_bytes(), b"Hello");
        assert_eq!(unsafe { CStr::from_ptr(hello[1]) }.to_bytes(), b"World");
        assert_eq!(hello[2], std::ptr::null());
    }

    #[test]
    #[should_panic]
    fn cstr_arr_unhappy_n1() {
        let mut cbuf = [0u8; 25];
        let _ = cstr_arr_from_str_slice::<1>(&["Hello"], &mut cbuf);
    }

    #[test]
    fn cstr_arr_unhappy_n_too_small() {
        let mut cbuf = [0u8; 25];
        assert!(cstr_arr_from_str_slice::<2>(&["Hello", "World"], &mut cbuf).is_err());
    }

    #[test]
    #[should_panic]
    fn cstr_arr_unhappy_cbuf_too_small() {
        let mut cbuf = [0u8; 12];
        assert!(cstr_arr_from_str_slice::<3>(&["Hello", "World"], &mut cbuf).is_err());
    }
}
