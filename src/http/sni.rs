use std::ffi::CStr;
use std::ptr;
use esp_idf_sys::*;
use log::*;

// Workaround for unstable feature 'trait_alias'
pub trait SNICB<'a>: FnMut(&'a str) -> SNIResult<'a> { }

// Workaround for unstable feature 'trait_alias'
impl<'a, T> SNICB<'a> for T
    where T: FnMut(&'a str) -> SNIResult<'a> {
}

pub struct HandshakeServerCertificate<'a> {
    pub pk: &'a mut mbedtls_pk_context,
    pub cert: &'a mut mbedtls_x509_crt,
}

pub struct HandshakeCertifiacteAuthority<'a> {
    pub ca: &'a mut mbedtls_x509_crt,
    pub crl: &'a mut mbedtls_x509_crl,
}

pub struct HandshakeVerifyMode(c_types::c_int);

pub struct SNIResult<'a> {
    server_certificate: Option<HandshakeServerCertificate<'a>>,
    certificate_authority: Option<HandshakeCertifiacteAuthority<'a>>,
    verify_mode: Option<HandshakeVerifyMode>
}

impl<'a> SNIResult<'a> {
    pub fn new() -> SNIResult<'a> { SNIResult { server_certificate: None, certificate_authority: None, verify_mode: None }}

    pub fn set_hs_server_certficate(mut self, pk: &'a mut mbedtls_pk_context, cert: &'a mut mbedtls_x509_crt) -> SNIResult<'a> {
        self.server_certificate = Some(HandshakeServerCertificate { pk, cert });
        self
    }

    pub fn set_hs_certificate_authority(mut self, ca: &'a mut mbedtls_x509_crt, crl: &'a mut mbedtls_x509_crl) -> SNIResult<'a> {
        self.certificate_authority = Some(HandshakeCertifiacteAuthority { ca, crl });
        self
    }

    pub fn set_hs_verify_mode(mut self, verify_mode: u32) -> SNIResult<'a> {
        self.verify_mode = Some(HandshakeVerifyMode(verify_mode as _));
        self
    }
}

unsafe extern "C" fn f_rng(_arg: *mut c_types::c_void, ptr: *mut u8 , bytes: u32) -> i32 {
    esp_fill_random(ptr as _, bytes);
    bytes as _
}

pub(crate) unsafe extern "C" fn sni_trampoline<'a>(p_info: *mut c_types::c_void, ssl: *mut mbedtls_ssl_context, name: *const c_types::c_uchar, _len: c_types::c_uint) -> esp_err_t
{
    let cb = &mut *(p_info as *mut Box<dyn SNICB<'a>>);

    let name = CStr::from_ptr(name as _).to_str().unwrap();

    let SNIResult { server_certificate, certificate_authority, verify_mode } = cb(name);

    if let Some(HandshakeServerCertificate { pk, cert }) = server_certificate {
        if let Err(err) = esp!(mbedtls_pk_check_pair(&mut cert.pk, pk, Some(f_rng), ptr::null_mut())) {
            error!("Certificate and private key supplied by the SNI callback do not match: {:?}", err);
            return err.code()
        };

        if let Err(err) = esp!(mbedtls_ssl_set_hs_own_cert(ssl, cert, pk)) {
            error!("Could not set handshake certificate and private key: {:?}", err);
            return err.code()
        };
    };

    if let Some(HandshakeCertifiacteAuthority { ca, crl }) = certificate_authority {
        mbedtls_ssl_set_hs_ca_chain(ssl, ca, crl)
    };

    if let Some(HandshakeVerifyMode(authmode)) = verify_mode {
        mbedtls_ssl_set_hs_authmode(ssl, authmode)
    };

    return ESP_OK;
}
