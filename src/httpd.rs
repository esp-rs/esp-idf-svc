//! HTTP server
//!
//! The HTTP Server component provides an ability for running a lightweight web
//! server on ESP32.
#![allow(deprecated)]

use core::{ffi::*, marker::PhantomData, ptr};

extern crate alloc;
use alloc::vec;

use std::io;

use ::anyhow::anyhow;

use ::log::{info, log, Level};

use embedded_svc::httpd::*;

use esp_idf_sys::esp;
use esp_idf_sys::*;

use crate::private::cstr::*;

struct IdfRequest<'r>(
    *mut esp_idf_sys::httpd_req_t,
    PhantomData<&'r esp_idf_sys::httpd_req_t>,
);

impl<'r> IdfRequest<'r> {
    fn send(&mut self, response: Response) -> Result<()> {
        let mut status_string = response.status.to_string();
        if let Some(message) = response.status_message {
            status_string.push(' ');
            status_string.push_str(message.as_str());
        }

        let c_status = to_cstring_arg(status_string.as_str())?;

        esp!(unsafe { esp_idf_sys::httpd_resp_set_status(self.0, c_status.as_ptr()) })?;

        let mut c_headers: std::vec::Vec<(CString, CString)> = vec![];
        let mut c_content_type: Option<CString> = None;
        let mut content_len: Option<usize> = None;

        for (key, value) in response.headers {
            if key.as_str().eq_ignore_ascii_case("Content-Type") {
                c_content_type = Some(to_cstring_arg(value.as_str()).unwrap());
            } else if key.as_str().eq_ignore_ascii_case("Content-Length") {
                content_len = Some(
                    value
                        .as_str()
                        .parse::<usize>()
                        .map_err(|_| anyhow!("Header Content-Length is invalid: {}", value))?,
                );
            } else {
                c_headers.push((
                    to_cstring_arg(key.as_str()).unwrap(),
                    to_cstring_arg(value.as_str()).unwrap(),
                ))
            }
        }

        if let Some(c_content_type) = c_content_type.as_ref() {
            esp!(unsafe { esp_idf_sys::httpd_resp_set_type(self.0, c_content_type.as_ptr()) })?
        }

        for (c_field, c_value) in &c_headers {
            esp!(unsafe {
                esp_idf_sys::httpd_resp_set_hdr(self.0, c_field.as_ptr(), c_value.as_ptr())
            })?;
        }

        match response.body {
            Body::Empty => self.send_body_bytes(content_len, &[]),
            Body::Bytes(vec) => self.send_body_bytes(content_len, vec.as_slice()),
            Body::Read(size, mut r) => self.send_body_read(content_len, size, &mut r),
        }
    }

    fn send_body_bytes(&mut self, _size: Option<usize>, data: &[u8]) -> anyhow::Result<()> {
        esp!(unsafe {
            esp_idf_sys::httpd_resp_send(self.0, data.as_ptr().cast(), data.len() as isize)
        })
        .map_err(Into::into)
    }

    fn send_body_read<R: io::Read>(
        &mut self,
        _content_len: Option<usize>,
        _size: Option<usize>,
        r: &mut R,
    ) -> anyhow::Result<()> {
        let mut buf = [0; 256];

        loop {
            let len = r.read(&mut buf)?;

            esp!(unsafe {
                esp_idf_sys::httpd_resp_send_chunk(self.0, buf.as_ptr().cast(), buf.len() as isize)
            })?;

            if len == 0 {
                break;
            }
        }
        Ok(())
    }
}

impl<'r> RequestDelegate for IdfRequest<'r> {
    fn header(&self, name: &str) -> Option<String> {
        if let Ok(c_str) = to_cstring_arg(name) {
            unsafe {
                match esp_idf_sys::httpd_req_get_hdr_value_len(self.0, c_str.as_ptr()) {
                    0 => None,
                    len => {
                        let mut buf: vec::Vec<u8> = Vec::with_capacity(len + 1);

                        esp_nofail!(esp_idf_sys::httpd_req_get_hdr_value_str(
                            self.0,
                            c_str.as_ptr(),
                            buf.as_mut_ptr().cast(),
                            len + 1
                        ));

                        buf.set_len(len + 1);

                        Some(std::str::from_utf8_unchecked(&buf[..len]).into())
                    }
                }
            }
        } else {
            None
        }
    }

    fn query_string(&self) -> Option<String> {
        unsafe {
            match esp_idf_sys::httpd_req_get_url_query_len(self.0) {
                0 => None,
                len => {
                    let mut buf: vec::Vec<u8> = Vec::with_capacity(len + 1);

                    esp_nofail!(esp_idf_sys::httpd_req_get_url_query_str(
                        self.0,
                        buf.as_mut_ptr().cast(),
                        len + 1
                    ));

                    buf.set_len(len + 1);

                    Some(std::str::from_utf8_unchecked(&buf[..len]).into())
                }
            }
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        unsafe {
            let len = esp_idf_sys::httpd_req_recv(self.0, buf.as_mut_ptr().cast(), buf.len());

            if len < 0 {
                Err(match len {
                    esp_idf_sys::HTTPD_SOCK_ERR_INVALID => io::ErrorKind::InvalidInput,
                    esp_idf_sys::HTTPD_SOCK_ERR_TIMEOUT => io::ErrorKind::TimedOut,
                    esp_idf_sys::HTTPD_SOCK_ERR_FAIL => io::ErrorKind::Other,
                    _ => io::ErrorKind::Other,
                }
                .into())
            } else {
                Ok(len as usize)
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Configuration {
    pub http_port: u16,
    pub https_port: u16,
    pub max_uri_handlers: u16,
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration {
            http_port: 80,
            https_port: 443,
            max_uri_handlers: 8,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum HttpdError {
    InternalServerError,
    MethodNotImplemented,
    VersionNotSupported,
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    ReqTimeout,
    LengthRequired,
    UriTooLong,
    ReqHdrFieldsTooLarge,
}

impl From<httpd_err_code_t> for HttpdError {
    #[allow(non_upper_case_globals)]
    fn from(from: httpd_err_code_t) -> Self {
        match from {
            httpd_err_code_t_HTTPD_500_INTERNAL_SERVER_ERROR => HttpdError::InternalServerError,
            httpd_err_code_t_HTTPD_501_METHOD_NOT_IMPLEMENTED => HttpdError::MethodNotImplemented,
            httpd_err_code_t_HTTPD_505_VERSION_NOT_SUPPORTED => HttpdError::VersionNotSupported,
            httpd_err_code_t_HTTPD_400_BAD_REQUEST => HttpdError::BadRequest,
            httpd_err_code_t_HTTPD_401_UNAUTHORIZED => HttpdError::Unauthorized,
            httpd_err_code_t_HTTPD_403_FORBIDDEN => HttpdError::Forbidden,
            httpd_err_code_t_HTTPD_404_NOT_FOUND => HttpdError::NotFound,
            httpd_err_code_t_HTTPD_405_METHOD_NOT_ALLOWED => HttpdError::MethodNotAllowed,
            httpd_err_code_t_HTTPD_408_REQ_TIMEOUT => HttpdError::ReqTimeout,
            httpd_err_code_t_HTTPD_411_LENGTH_REQUIRED => HttpdError::LengthRequired,
            httpd_err_code_t_HTTPD_414_URI_TOO_LONG => HttpdError::UriTooLong,
            httpd_err_code_t_HTTPD_431_REQ_HDR_FIELDS_TOO_LARGE => HttpdError::ReqHdrFieldsTooLarge,
            _ => unimplemented!("all httpd errors should be covered"),
        }
    }
}

impl Into<httpd_err_code_t> for HttpdError {
    #[allow(non_upper_case_globals)]
    fn into(self) -> httpd_err_code_t {
        match self {
            HttpdError::InternalServerError => httpd_err_code_t_HTTPD_500_INTERNAL_SERVER_ERROR,
            HttpdError::MethodNotImplemented => httpd_err_code_t_HTTPD_501_METHOD_NOT_IMPLEMENTED,
            HttpdError::VersionNotSupported => httpd_err_code_t_HTTPD_505_VERSION_NOT_SUPPORTED,
            HttpdError::BadRequest => httpd_err_code_t_HTTPD_400_BAD_REQUEST,
            HttpdError::Unauthorized => httpd_err_code_t_HTTPD_401_UNAUTHORIZED,
            HttpdError::Forbidden => httpd_err_code_t_HTTPD_403_FORBIDDEN,
            HttpdError::NotFound => httpd_err_code_t_HTTPD_404_NOT_FOUND,
            HttpdError::MethodNotAllowed => httpd_err_code_t_HTTPD_405_METHOD_NOT_ALLOWED,
            HttpdError::ReqTimeout => httpd_err_code_t_HTTPD_408_REQ_TIMEOUT,
            HttpdError::LengthRequired => httpd_err_code_t_HTTPD_411_LENGTH_REQUIRED,
            HttpdError::UriTooLong => httpd_err_code_t_HTTPD_414_URI_TOO_LONG,
            HttpdError::ReqHdrFieldsTooLarge => httpd_err_code_t_HTTPD_431_REQ_HDR_FIELDS_TOO_LARGE,
        }
    }
}

pub struct ErrorHandler {
    error: HttpdError,
    // handler: Box<dyn Fn(Request, HttpdError) -> Result<Response>>,
    handler: unsafe extern "C" fn(req: *mut httpd_req_t, error: httpd_err_code_t) -> esp_err_t,
}

pub struct ServerRegistry {
    registry: registry::MiddlewareRegistry,
    error_handlers: Vec<ErrorHandler>,
}

impl ServerRegistry {
    pub fn new() -> Self {
        ServerRegistry {
            registry: Default::default(),
            error_handlers: Vec::new(),
        }
    }

    pub fn error_handler(
        mut self,
        error: HttpdError,
        f: unsafe extern "C" fn(req: *mut httpd_req_t, error: httpd_err_code_t) -> esp_err_t,
    ) -> ServerRegistry {
        self.error_handlers.push(ErrorHandler {
            error,
            handler: f,
        });
        ServerRegistry {
            registry: self.registry,
            error_handlers: self.error_handlers,
        }
    }

    pub fn start(self, configuration: &Configuration) -> Result<Server> {
        let mut server = Server::new(configuration)?;

        for handler in self.registry.apply_middleware() {
            server.register(handler)?;
        }

        for error_handler in self.error_handlers {
            server.register_error_handler(error_handler)?;
        }

        Ok(server)
    }
}

impl Default for ServerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl registry::Registry for ServerRegistry {
    fn handler(self, handler: Handler) -> Result<Self> {
        Ok(Self {
            registry: self.registry.handler(handler)?,
            ..self
        })
    }

    fn middleware(self, middleware: Middleware) -> Result<Self> {
        Ok(Self {
            registry: self.registry.middleware(middleware)?,
            ..self
        })
    }
}

pub struct Server {
    sd: esp_idf_sys::httpd_handle_t,
    registrations: Vec<(CString, esp_idf_sys::httpd_uri_t)>,
}

impl Server {
    fn new(conf: &Configuration) -> Result<Self> {
        let config =
            Self::default_configuration(conf.http_port, conf.https_port, conf.max_uri_handlers);

        let mut handle: esp_idf_sys::httpd_handle_t = ptr::null_mut();
        let handle_ref = &mut handle;

        esp!(unsafe { esp_idf_sys::httpd_start(handle_ref, &config) })?;

        info!("Started Httpd IDF server with config {:?}", conf);

        Ok(Server {
            sd: handle,
            registrations: vec![],
        })
    }

    fn register(&mut self, handler: Handler) -> Result<()> {
        let c_str = to_cstring_arg(handler.uri().as_ref())?;
        let method = handler.method();

        #[allow(clippy::needless_update)]
        let conf = esp_idf_sys::httpd_uri_t {
            uri: c_str.as_ptr(),
            method: Self::get_httpd_method(method),
            user_ctx: Box::into_raw(Box::new(handler.handler())).cast(),
            handler: Some(Server::handle),
            ..Default::default()
        };

        esp!(unsafe { esp_idf_sys::httpd_register_uri_handler(self.sd, &conf) })?;

        info!(
            "Registered Httpd IDF server handler {:?} for URI \"{}\"",
            method,
            c_str.to_str().unwrap()
        );

        self.registrations.push((c_str, conf));

        Ok(())
    }

    fn unregister(&mut self, uri: CString, conf: esp_idf_sys::httpd_uri_t) -> Result<()> {
        unsafe {
            esp!(esp_idf_sys::httpd_unregister_uri_handler(
                self.sd,
                uri.as_ptr(),
                conf.method
            ))?;

            let _drop =
                Box::from_raw(conf.user_ctx as *mut Box<dyn Fn(Request) -> Result<Response>>);
        };

        info!(
            "Unregistered Httpd IDF server handler {:?} for URI \"{}\"",
            conf.method,
            uri.to_str().unwrap()
        );

        Ok(())
    }

    fn register_error_handler(&mut self, handler: ErrorHandler) -> Result<()> {
        esp!(unsafe { esp_idf_sys::httpd_register_err_handler(self.sd, handler.error.into(), Some(handler.handler)) })?;

        info!(
            "Registered Httpd IDF error handler for Error \"{:?}\"",
            handler.error
        );

        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        if !self.sd.is_null() {
            while let Some((uri, registration)) = self.registrations.pop() {
                self.unregister(uri, registration)?;
            }

            esp!(unsafe { esp_idf_sys::httpd_stop(self.sd) })?;

            self.sd = ptr::null_mut();
        }

        info!("Httpd IDF server stopped");

        Ok(())
    }

    unsafe extern "C" fn handle(rd: *mut esp_idf_sys::httpd_req_t) -> c_int {
        let handler = ((*rd).user_ctx as *mut Box<dyn Fn(Request) -> Result<Response>>)
            .as_ref()
            .unwrap();

        let idf_request = IdfRequest(rd, PhantomData);
        info!(
            "About to handle query string {:?}",
            idf_request.query_string()
        );

        let (response, err) = match handler(Request::new(
            Box::new(idf_request),
            StateMap::new(),
            None,
            None,
        )) {
            Ok(response) => (response, false),
            Err(err) => (err.into(), true),
        };

        let mut idf_request_response = IdfRequest(rd, PhantomData);

        log!(
            if err { Level::Warn } else { Level::Info },
            "Request handled with status {} ({:?})",
            &response.status,
            &response.status_message
        );

        match idf_request_response.send(response) {
            Ok(_) => esp_idf_sys::ESP_OK,
            Err(_) => esp_idf_sys::ESP_FAIL,
        }
    }

    fn get_httpd_method(m: Method) -> c_uint {
        match m {
            Method::Get => esp_idf_sys::http_method_HTTP_GET,
            Method::Post => esp_idf_sys::http_method_HTTP_POST,
            Method::Delete => esp_idf_sys::http_method_HTTP_DELETE,
            Method::Head => esp_idf_sys::http_method_HTTP_HEAD,
            Method::Put => esp_idf_sys::http_method_HTTP_PUT,
            Method::Connect => esp_idf_sys::http_method_HTTP_CONNECT,
            Method::Options => esp_idf_sys::http_method_HTTP_OPTIONS,
            Method::Trace => esp_idf_sys::http_method_HTTP_TRACE,
            Method::Copy => esp_idf_sys::http_method_HTTP_COPY,
            Method::Lock => esp_idf_sys::http_method_HTTP_LOCK,
            Method::MkCol => esp_idf_sys::http_method_HTTP_MKCOL,
            Method::Move => esp_idf_sys::http_method_HTTP_MOVE,
            Method::Propfind => esp_idf_sys::http_method_HTTP_PROPFIND,
            Method::Proppatch => esp_idf_sys::http_method_HTTP_PROPPATCH,
            Method::Search => esp_idf_sys::http_method_HTTP_SEARCH,
            Method::Unlock => esp_idf_sys::http_method_HTTP_UNLOCK,
            Method::Bind => esp_idf_sys::http_method_HTTP_BIND,
            Method::Rebind => esp_idf_sys::http_method_HTTP_REBIND,
            Method::Unbind => esp_idf_sys::http_method_HTTP_UNBIND,
            Method::Acl => esp_idf_sys::http_method_HTTP_ACL,
            Method::Report => esp_idf_sys::http_method_HTTP_REPORT,
            Method::MkActivity => esp_idf_sys::http_method_HTTP_MKACTIVITY,
            Method::Checkout => esp_idf_sys::http_method_HTTP_CHECKOUT,
            Method::Merge => esp_idf_sys::http_method_HTTP_MERGE,
            Method::MSearch => esp_idf_sys::http_method_HTTP_MSEARCH,
            Method::Notify => esp_idf_sys::http_method_HTTP_NOTIFY,
            Method::Subscribe => esp_idf_sys::http_method_HTTP_SUBSCRIBE,
            Method::Unsubscribe => esp_idf_sys::http_method_HTTP_UNSUBSCRIBE,
            Method::Patch => esp_idf_sys::http_method_HTTP_PATCH,
            Method::Purge => esp_idf_sys::http_method_HTTP_PURGE,
            Method::MkCalendar => esp_idf_sys::http_method_HTTP_MKCALENDAR,
            Method::Link => esp_idf_sys::http_method_HTTP_LINK,
            Method::Unlink => esp_idf_sys::http_method_HTTP_UNLINK,
        }
    }

    /// Copied from the definition of HTTPD_DEFAULT_CONFIG() in http_server.h/https_server.h
    #[allow(clippy::needless_update)]
    fn default_configuration(
        http_port: u16,
        https_port: u16,
        max_uri_handlers: u16,
    ) -> esp_idf_sys::httpd_config_t {
        esp_idf_sys::httpd_config_t {
            task_priority: 5,
            stack_size: if https_port != 0 { 10240 } else { 4096 },
            core_id: std::i32::MAX,
            server_port: http_port,
            ctrl_port: 32768,
            max_open_sockets: if https_port != 0 { 4 } else { 7 },
            max_uri_handlers,
            max_resp_headers: 8,
            backlog_conn: 5,
            lru_purge_enable: https_port != 0,
            recv_wait_timeout: 5,
            send_wait_timeout: 5,
            global_user_ctx: ptr::null_mut(),
            global_user_ctx_free_fn: None,
            global_transport_ctx: ptr::null_mut(),
            global_transport_ctx_free_fn: None,
            open_fn: None,
            close_fn: None,
            uri_match_fn: None,
            // Latest 4.4 and master branches have options to control SO linger,
            // but these are not released yet so we cannot (yet) support these
            // conditionally
            ..Default::default()
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop().expect("Unable to stop the server cleanly");
    }
}
