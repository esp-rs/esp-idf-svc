//! Type safe abstraction for esp-tls

use std::{
    ffi::CStr,
    fmt::{Debug, Display},
    io::{self, Read, Write},
};

pub struct EspTls {
    raw: *mut esp_idf_sys::esp_tls,
}

impl EspTls {
    pub fn new(host: &str, port: i32, cfg: Config) -> Result<Self> {
        // TODO: where to put async version? seperate struct?
        let tls = unsafe { esp_idf_sys::esp_tls_init() };
        if tls == std::ptr::null_mut() {
            return Err(Error::BadAlloc);
        }
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

        let mut alpn_protos: Vec<*const i8>;
        if let Some(protos) = cfg.alpn_protos {
            alpn_protos = protos
                .iter()
                .map(|p| p.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            rcfg.alpn_protos = alpn_protos.as_mut_ptr();
        }

        rcfg.non_block = cfg.non_block;
        rcfg.use_secure_element = cfg.use_secure_element;
        rcfg.timeout_ms = cfg.timeout_ms as i32;
        rcfg.use_global_ca_store = cfg.use_global_ca_store;
        rcfg.common_name = cfg.common_name.as_ptr();
        rcfg.skip_common_name = cfg.skip_common_name;

        let mut raw_kac: esp_idf_sys::tls_keep_alive_cfg;
        if let Some(kac) = cfg.keep_alive_cfg {
            raw_kac = esp_idf_sys::tls_keep_alive_cfg {
                keep_alive_enable: kac.enable,
                keep_alive_idle: kac.idle.as_secs() as i32,
                keep_alive_interval: kac.interval.as_secs() as i32,
                keep_alive_count: kac.count as i32,
            };
            rcfg.keep_alive_cfg = &mut raw_kac as *mut _;
        }

        let mut raw_psk: esp_idf_sys::psk_key_hint;
        if let Some(psk) = cfg.psk_hint_key {
            raw_psk = esp_idf_sys::psk_key_hint {
                key: psk.key.as_ptr(),
                key_size: psk.key.len(),
                hint: psk.hint.as_ptr(),
            };
            rcfg.psk_hint_key = &mut raw_psk as *mut _;
        }

        rcfg.is_plain_tcp = cfg.is_plain_tcp;
        rcfg.if_name = std::ptr::null_mut();

        let ret = unsafe {
            esp_idf_sys::esp_tls_conn_new_sync(
                host.as_bytes().as_ptr() as *const i8,
                host.len() as i32,
                port,
                &rcfg,
                tls,
            )
        };

        if ret == 1 {
            Ok(EspTls { raw: tls })
        } else {
            unsafe {
                esp_idf_sys::esp_tls_conn_destroy(tls);
            }

            Err(Error::ConnectionNotEstablished)
        }
    }

    pub fn close(self) {}
}

impl Read for EspTls {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // cannot call esp_tls_conn_read bc it's inline
        //let ret = unsafe { esp_idf_sys::esp_tls_conn_read(self.raw, buf.as_mut_ptr(), buf.len()) };
        let esp_tls = unsafe { std::ptr::read_unaligned(self.raw) };
        let read_func = esp_tls.read.unwrap();
        let ret = unsafe { read_func(self.raw, buf.as_mut_ptr() as *mut i8, buf.len()) } as i32;
        // ESP docs treat 0 as error, but in Rust it's common to return 0 from `Read::read` to indicate eof
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, Error::ReadError(ret)))
        }
    }
}

impl Write for EspTls {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // cannot call esp_tls_conn_write bc it's inline
        //let ret = unsafe { esp_idf_sys::esp_tls_conn_write(self.raw, buf.as_ptr(), buf.len()) };
        let esp_tls = unsafe { std::ptr::read_unaligned(self.raw) };
        let write_func = esp_tls.write.unwrap();
        let ret = unsafe { write_func(self.raw, buf.as_ptr() as *const i8, buf.len()) } as i32;
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, Error::WriteError(ret)))
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for EspTls {
    fn drop(&mut self) {
        unsafe {
            esp_idf_sys::esp_tls_conn_destroy(self.raw);
        }
    }
}

pub struct Config<'a> {
    pub alpn_protos: Option<&'a [&'a CStr]>,
    pub ca_cert: Option<X509<'a>>,
    pub client_cert: Option<X509<'a>>,
    pub client_key: Option<X509<'a>>,
    pub client_key_password: Option<&'a CStr>,
    pub non_block: bool,
    pub use_secure_element: bool,
    pub timeout_ms: u32,
    pub use_global_ca_store: bool,
    pub common_name: &'a CStr,
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

#[derive(Clone, Debug)]
pub enum Error {
    BadAlloc,
    ConnectionNotEstablished,
    ConnectionClosed,
    ReadError(i32),
    WriteError(i32),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadAlloc => write!(f, "bad alloc"),
            Error::ConnectionNotEstablished => write!(f, "connection not established"),
            Error::ConnectionClosed => write!(f, "connection closed"),
            Error::ReadError(code) => write!(f, "read error {code}"),
            Error::WriteError(code) => write!(f, "write error {code}"),
        }
    }
}

impl std::error::Error for Error {}

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
