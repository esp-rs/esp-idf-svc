use esp_idf_sys::*;
use log::*;
use std::ffi::CStr;
use std::ptr;

// Workaround for unstable feature 'trait_alias'
pub trait CertSelectCallback<'a>: FnMut(&'a str) -> CallbackResult<'a> {}

// Workaround for unstable feature 'trait_alias'
impl<'a, T> CertSelectCallback<'a> for T where T: FnMut(&'a str) -> CallbackResult<'a> {}

pub struct HandshakeServerCertificate<'a> {
    pub pk: &'a mut mbedtls_pk_context,
    pub cert: &'a mut mbedtls_x509_crt,
}

pub struct HandshakeCertifiacteAuthority<'a> {
    pub ca: &'a mut mbedtls_x509_crt,
    pub crl: &'a mut mbedtls_x509_crl,
}

pub struct HandshakeVerifyMode(c_types::c_int);

pub struct CallbackResult<'a> {
    server_certificate: Option<HandshakeServerCertificate<'a>>,
    certificate_authority: Option<HandshakeCertifiacteAuthority<'a>>,
    verify_mode: Option<HandshakeVerifyMode>,
}

impl<'a> CallbackResult<'a> {
    pub fn new() -> CallbackResult<'a> {
        CallbackResult {
            server_certificate: None,
            certificate_authority: None,
            verify_mode: None,
        }
    }

    pub fn set_hs_server_certficate(
        mut self,
        pk: &'a mut mbedtls_pk_context,
        cert: &'a mut mbedtls_x509_crt,
    ) -> CallbackResult<'a> {
        self.server_certificate = Some(HandshakeServerCertificate { pk, cert });
        self
    }

    pub fn set_hs_certificate_authority(
        mut self,
        ca: &'a mut mbedtls_x509_crt,
        crl: &'a mut mbedtls_x509_crl,
    ) -> CallbackResult<'a> {
        self.certificate_authority = Some(HandshakeCertifiacteAuthority { ca, crl });
        self
    }

    pub fn set_hs_verify_mode(mut self, verify_mode: u32) -> CallbackResult<'a> {
        self.verify_mode = Some(HandshakeVerifyMode(verify_mode as _));
        self
    }
}

unsafe extern "C" fn f_rng(_arg: *mut c_types::c_void, ptr: *mut u8, bytes: u32) -> i32 {
    esp_fill_random(ptr as _, bytes);
    bytes as _
}

pub(crate) unsafe extern "C" fn cert_select_trampoline<'a>(
    ssl: *mut mbedtls_ssl_context,
) -> esp_err_t {
    // Need to use ->private_user_data as the getter function is static inline, and
    // bindgen can't generate bindings for static inline yet.
    // https://github.com/rust-lang/rust-bindgen/issues/1090

    let ssl_conf = (*ssl).private_conf;

    if ssl_conf == ptr::null_mut() {
        return ESP_ERR_INVALID_ARG;
    }

    let cb_ptr = (*ssl_conf).private_user_data.p;

    if cb_ptr == ptr::null_mut() {
        return ESP_ERR_INVALID_ARG;
    }

    let cb = &mut *(cb_ptr as *mut Box<dyn CertSelectCallback<'a>>);
    let mut namelen: u32 = 0;

    let name = mbedtls_ssl_get_hs_sni(ssl, &mut namelen);
    let name = CStr::from_ptr(name as _).to_str().unwrap();

    let CallbackResult {
        server_certificate,
        certificate_authority,
        verify_mode,
    } = cb(name);

    if let Some(HandshakeServerCertificate { pk, cert }) = server_certificate {
        if let Err(err) = esp!(mbedtls_pk_check_pair(
            &mut cert.pk,
            pk,
            Some(f_rng),
            ptr::null_mut()
        )) {
            error!(
                "Certificate and private key supplied by the SNI callback do not match: {:?}",
                err
            );
            return err.code();
        };

        if let Err(err) = esp!(mbedtls_ssl_set_hs_own_cert(ssl, cert, pk)) {
            error!(
                "Could not set handshake certificate and private key: {:?}",
                err
            );
            return err.code();
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
