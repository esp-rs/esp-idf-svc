//! MQTT protocol client
use core::convert::{TryFrom, TryInto};
use core::ffi::c_void;
use core::fmt::{self, Debug};
use core::{slice, time};

extern crate alloc;
use alloc::boxed::Box;
use alloc::sync::Arc;

use embedded_svc::mqtt::client::{self, ErrorType};

use crate::sys::*;

use crate::handle::RawHandle;
use crate::private::mutex::RawCondvar;

#[allow(unused_imports)]
pub use asyncify::*;

use crate::private::cstr::*;
use crate::tls::*;

pub use client::{
    Details, Event, InitialChunkData, Message, MessageId, MessageImpl, QoS, SubsequentChunkData,
};
pub use embedded_svc::utils::mqtt::client::{ConnState, ConnStateGuard, Connection, Postbox};

#[allow(unused_imports)]
pub use super::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MqttProtocolVersion {
    V3_1,
    V3_1_1,
}

impl From<MqttProtocolVersion> for esp_mqtt_protocol_ver_t {
    fn from(pv: MqttProtocolVersion) -> Self {
        match pv {
            MqttProtocolVersion::V3_1 => esp_mqtt_protocol_ver_t_MQTT_PROTOCOL_V_3_1,
            MqttProtocolVersion::V3_1_1 => esp_mqtt_protocol_ver_t_MQTT_PROTOCOL_V_3_1_1,
        }
    }
}

#[derive(Debug)]
pub struct LwtConfiguration<'a> {
    pub topic: &'a str,
    pub payload: &'a [u8],
    pub qos: client::QoS,
    pub retain: bool,
}

#[derive(Debug)]
pub struct MqttClientConfiguration<'a> {
    pub protocol_version: Option<MqttProtocolVersion>,

    pub client_id: Option<&'a str>,

    pub connection_refresh_interval: time::Duration,
    pub keep_alive_interval: Option<time::Duration>,
    pub reconnect_timeout: Option<time::Duration>,
    pub network_timeout: time::Duration,

    pub lwt: Option<LwtConfiguration<'a>>,

    pub disable_clean_session: bool,

    pub task_prio: u8,
    pub task_stack: usize,
    pub buffer_size: usize,
    pub out_buffer_size: usize,

    pub username: Option<&'a str>,
    pub password: Option<&'a str>,

    pub use_global_ca_store: bool,
    pub skip_cert_common_name_check: bool,
    #[cfg(not(esp_idf_version = "4.3"))]
    pub crt_bundle_attach: Option<unsafe extern "C" fn(conf: *mut c_void) -> esp_err_t>,

    pub server_certificate: Option<X509<'static>>,

    pub client_certificate: Option<X509<'static>>,
    pub private_key: Option<X509<'static>>,
    pub private_key_password: Option<&'a str>,

    #[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
    pub psk: Option<Psk<'a>>,
    // pub alpn_protos: &'a [&'a str],
    // pub use_secure_element: bool,
    // void *ds_data;                          /*!< carrier of handle for digital signature parameters */
}

impl<'a> Default for MqttClientConfiguration<'a> {
    fn default() -> Self {
        Self {
            protocol_version: None,

            client_id: None,

            connection_refresh_interval: time::Duration::from_secs(0),
            keep_alive_interval: Some(time::Duration::from_secs(0)),
            reconnect_timeout: Some(time::Duration::from_secs(0)),
            network_timeout: time::Duration::from_secs(0),

            lwt: None,

            disable_clean_session: false,

            task_prio: 0,
            task_stack: 0,
            buffer_size: 0,
            out_buffer_size: 0,

            username: None,
            password: None,

            use_global_ca_store: false,
            skip_cert_common_name_check: false,

            #[cfg(not(esp_idf_version = "4.3"))]
            crt_bundle_attach: Default::default(),

            server_certificate: None,

            client_certificate: None,
            private_key: None,
            private_key_password: None,

            #[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
            psk: None,
        }
    }
}

#[cfg(esp_idf_version_major = "4")]
impl<'a> TryFrom<&'a MqttClientConfiguration<'a>>
    for (esp_mqtt_client_config_t, RawCstrs, Option<TlsPsk>)
{
    type Error = EspError;

    fn try_from(conf: &'a MqttClientConfiguration<'a>) -> Result<Self, Self::Error> {
        let mut cstrs = RawCstrs::new();

        let mut c_conf = esp_mqtt_client_config_t {
            protocol_ver: if let Some(protocol_version) = conf.protocol_version {
                protocol_version.into()
            } else {
                esp_mqtt_protocol_ver_t_MQTT_PROTOCOL_UNDEFINED
            },
            client_id: cstrs.as_nptr(conf.client_id)?,

            refresh_connection_after_ms: conf.connection_refresh_interval.as_millis() as _,
            network_timeout_ms: conf.network_timeout.as_millis() as _,

            disable_clean_session: conf.disable_clean_session as _,

            task_prio: conf.task_prio as _,
            task_stack: conf.task_stack as _,
            buffer_size: conf.buffer_size as _,
            out_buffer_size: conf.out_buffer_size as _,

            username: cstrs.as_nptr(conf.username)?,
            password: cstrs.as_nptr(conf.password)?,

            use_global_ca_store: conf.use_global_ca_store,
            skip_cert_common_name_check: conf.skip_cert_common_name_check,
            #[cfg(not(esp_idf_version = "4.3"))]
            crt_bundle_attach: conf.crt_bundle_attach,

            ..Default::default()
        };

        if let Some(keep_alive_interval) = conf.keep_alive_interval {
            c_conf.keepalive = keep_alive_interval.as_secs() as _;
            c_conf.disable_keepalive = false;
        } else {
            c_conf.disable_keepalive = true;
        }

        if let Some(reconnect_timeout) = conf.reconnect_timeout {
            c_conf.reconnect_timeout_ms = reconnect_timeout.as_millis() as _;
            c_conf.disable_auto_reconnect = false;
        } else {
            c_conf.disable_auto_reconnect = true;
        }

        if let Some(lwt) = conf.lwt.as_ref() {
            c_conf.lwt_topic = cstrs.as_ptr(lwt.topic)?;
            c_conf.lwt_msg = lwt.payload.as_ptr() as _;
            c_conf.lwt_msg_len = lwt.payload.len() as _;
            c_conf.lwt_qos = lwt.qos as _;
            c_conf.lwt_retain = lwt.retain as _;
        }

        if let Some(cert) = conf.server_certificate {
            c_conf.cert_pem = cert.as_esp_idf_raw_ptr() as _;
            c_conf.cert_len = cert.as_esp_idf_raw_len();
        }

        if let (Some(cert), Some(private_key)) = (conf.client_certificate, conf.private_key) {
            c_conf.client_cert_pem = cert.as_esp_idf_raw_ptr() as _;
            c_conf.client_cert_len = cert.as_esp_idf_raw_len();

            c_conf.client_key_pem = private_key.as_esp_idf_raw_ptr() as _;
            c_conf.client_key_len = private_key.as_esp_idf_raw_len();

            if let Some(pass) = conf.private_key_password {
                c_conf.clientkey_password = pass.as_ptr() as _;
                c_conf.clientkey_password_len = pass.len() as _;
            }
        }

        #[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
        let tls_psk_conf = conf.psk.as_ref().map(|psk| psk.try_into()).transpose()?;
        #[cfg(not(all(esp_idf_esp_tls_psk_verification, feature = "alloc")))]
        let tls_psk_conf = None;

        Ok((c_conf, cstrs, tls_psk_conf))
    }
}

#[allow(clippy::needless_update)]
#[cfg(not(esp_idf_version_major = "4"))]
impl<'a> TryFrom<&'a MqttClientConfiguration<'a>>
    for (esp_mqtt_client_config_t, RawCstrs, Option<TlsPsk>)
{
    type Error = EspError;

    fn try_from(conf: &'a MqttClientConfiguration<'a>) -> Result<Self, EspError> {
        let mut cstrs = RawCstrs::new();

        #[allow(clippy::needless_update)]
        let mut c_conf = esp_mqtt_client_config_t {
            broker: esp_mqtt_client_config_t_broker_t {
                verification: esp_mqtt_client_config_t_broker_t_verification_t {
                    use_global_ca_store: conf.use_global_ca_store,
                    skip_cert_common_name_check: conf.skip_cert_common_name_check,
                    crt_bundle_attach: conf.crt_bundle_attach,
                    ..Default::default()
                },
                ..Default::default()
            },
            credentials: esp_mqtt_client_config_t_credentials_t {
                client_id: cstrs.as_nptr(conf.client_id)?,
                set_null_client_id: conf.client_id.is_none(),
                username: cstrs.as_nptr(conf.username)?,
                authentication: esp_mqtt_client_config_t_credentials_t_authentication_t {
                    password: cstrs.as_nptr(conf.password)?,
                    ..Default::default()
                },
                ..Default::default()
            },
            session: esp_mqtt_client_config_t_session_t {
                protocol_ver: if let Some(protocol_version) = conf.protocol_version {
                    protocol_version.into()
                } else {
                    esp_mqtt_protocol_ver_t_MQTT_PROTOCOL_UNDEFINED
                },
                disable_clean_session: conf.disable_clean_session as _,
                ..Default::default()
            },
            network: esp_mqtt_client_config_t_network_t {
                refresh_connection_after_ms: conf.connection_refresh_interval.as_millis() as _,
                timeout_ms: conf.network_timeout.as_millis() as _,
                ..Default::default()
            },
            task: esp_mqtt_client_config_t_task_t {
                priority: conf.task_prio as _,
                stack_size: conf.task_stack as _,
                ..Default::default()
            },
            buffer: esp_mqtt_client_config_t_buffer_t {
                size: conf.buffer_size as _,
                out_size: conf.out_buffer_size as _,
                ..Default::default()
            },
            ..Default::default()
        };

        if let Some(keep_alive_interval) = conf.keep_alive_interval {
            c_conf.session.keepalive = keep_alive_interval.as_secs() as _;
            c_conf.session.disable_keepalive = false;
        } else {
            c_conf.session.disable_keepalive = true;
        }

        if let Some(reconnect_timeout) = conf.reconnect_timeout {
            c_conf.network.reconnect_timeout_ms = reconnect_timeout.as_millis() as _;
            c_conf.network.disable_auto_reconnect = false;
        } else {
            c_conf.network.disable_auto_reconnect = true;
        }

        if let Some(lwt) = conf.lwt.as_ref() {
            c_conf.session.last_will = esp_mqtt_client_config_t_session_t_last_will_t {
                topic: cstrs.as_ptr(lwt.topic)?,
                msg: lwt.payload.as_ptr() as _,
                msg_len: lwt.payload.len() as _,
                qos: lwt.qos as _,
                retain: lwt.retain as _,
                ..Default::default()
            };
        }

        if let Some(cert) = conf.server_certificate {
            c_conf.broker.verification.certificate = cert.as_esp_idf_raw_ptr() as _;
            c_conf.broker.verification.certificate_len = cert.as_esp_idf_raw_len();
        }

        if let (Some(cert), Some(private_key)) = (conf.client_certificate, conf.private_key) {
            c_conf.credentials.authentication.certificate = cert.as_esp_idf_raw_ptr() as _;
            c_conf.credentials.authentication.certificate_len = cert.as_esp_idf_raw_len();

            c_conf.credentials.authentication.key = private_key.as_esp_idf_raw_ptr() as _;
            c_conf.credentials.authentication.key_len = private_key.as_esp_idf_raw_len();

            if let Some(pass) = conf.private_key_password {
                c_conf.credentials.authentication.key_password = pass.as_ptr() as _;
                c_conf.credentials.authentication.key_password_len = pass.len() as _;
            }
        }

        #[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
        let tls_psk_conf = conf.psk.as_ref().map(|psk| psk.try_into()).transpose()?;
        #[cfg(not(all(esp_idf_esp_tls_psk_verification, feature = "alloc")))]
        let tls_psk_conf = None;

        Ok((c_conf, cstrs, tls_psk_conf))
    }
}

struct UnsafeCallback<'a>(*mut Box<dyn FnMut(esp_mqtt_event_handle_t) + Send + 'a>);

impl<'a> UnsafeCallback<'a> {
    fn from(boxed: &mut Box<Box<dyn FnMut(esp_mqtt_event_handle_t) + Send + 'a>>) -> Self {
        Self(boxed.as_mut())
    }

    unsafe fn from_ptr(ptr: *mut c_void) -> Self {
        Self(ptr as *mut _)
    }

    fn as_ptr(&self) -> *mut c_void {
        self.0 as *mut _
    }

    unsafe fn call(&self, data: esp_mqtt_event_handle_t) {
        let reference = self.0.as_mut().unwrap();

        (reference)(data);
    }
}

pub struct EspMqttClient<'a, S = ()> {
    raw_client: esp_mqtt_client_handle_t,
    conn_state_guard: Option<Arc<ConnStateGuard<RawCondvar, S>>>,
    _boxed_raw_callback: Box<dyn FnMut(esp_mqtt_event_handle_t) + Send + 'a>,
    _tls_psk_conf: Option<TlsPsk>,
}

impl<'a, S> RawHandle for EspMqttClient<'a, S> {
    type Handle = esp_mqtt_client_handle_t;

    fn handle(&self) -> Self::Handle {
        self.raw_client
    }
}

impl EspMqttClient<'static, ConnState<MessageImpl, EspError>> {
    pub fn new_with_conn(
        url: &str,
        conf: &MqttClientConfiguration,
    ) -> Result<(Self, Connection<RawCondvar, MessageImpl, EspError>), EspError>
    where
        Self: Sized,
    {
        Self::new_with_converting_conn(url, conf, move |r| {
            r.as_ref()
                .map(|event| event.transform_received(MessageImpl::new))
                .map_err(|e| *e)
        })
    }
}

impl<M, E> EspMqttClient<'static, ConnState<M, E>>
where
    M: Send + 'static,
    E: Debug + Send + 'static,
{
    pub fn new_with_converting_conn<F>(
        url: &str,
        conf: &MqttClientConfiguration,
        converter: F,
    ) -> Result<(Self, Connection<RawCondvar, M, E>), EspError>
    where
        F: for<'b> FnMut(
                &'b Result<client::Event<EspMqttMessage<'b>>, EspError>,
            ) -> Result<client::Event<M>, E>
            + Send
            + 'static,
        Self: Sized,
    {
        Self::internal_new_with_converting_conn(url, conf, converter)
    }
}

impl<'a, M, E> EspMqttClient<'a, ConnState<M, E>>
where
    M: Send + 'static,
    E: Debug + Send + 'static,
{
    /// # Safety
    ///
    /// This method - in contrast to method `new_with_converting_conn` - allows the user to pass
    /// a non-static callback/closure. This enables users to borrow
    /// - in the closure - variables that live on the stack - or more generally - in the same
    /// scope where the service is created.
    ///
    /// HOWEVER: care should be taken NOT to call `core::mem::forget()` on the service,
    /// as that would immediately lead to an UB (crash).
    /// Also note that forgetting the service might happen with `Rc` and `Arc`
    /// when circular references are introduced: https://github.com/rust-lang/rust/issues/24456
    ///
    /// The reason is that the closure is actually sent to a hidden ESP IDF thread.
    /// This means that if the service is forgotten, Rust is free to e.g. unwind the stack
    /// and the closure now owned by this other thread will end up with references to variables that no longer exist.
    ///
    /// The destructor of the service takes care - prior to the service being dropped and e.g.
    /// the stack being unwind - to remove the closure from the hidden thread and destroy it.
    /// Unfortunately, when the service is forgotten, the un-subscription does not happen
    /// and invalid references are left dangling.
    ///
    /// This "local borrowing" will only be possible to express in a safe way once/if `!Leak` types
    /// are introduced to Rust (i.e. the impossibility to "forget" a type and thus not call its destructor).
    pub unsafe fn new_nonstatic_with_converting_conn<F>(
        url: &str,
        conf: &MqttClientConfiguration,
        converter: F,
    ) -> Result<(Self, Connection<RawCondvar, M, E>), EspError>
    where
        F: for<'b> FnMut(
                &'b Result<client::Event<EspMqttMessage<'b>>, EspError>,
            ) -> Result<client::Event<M>, E>
            + Send
            + 'a,
        Self: Sized,
    {
        Self::internal_new_with_converting_conn(url, conf, converter)
    }

    fn internal_new_with_converting_conn(
        url: &str,
        conf: &MqttClientConfiguration,
        mut converter: impl for<'b> FnMut(
                &'b Result<client::Event<EspMqttMessage<'b>>, EspError>,
            ) -> Result<client::Event<M>, E>
            + Send
            + 'a,
    ) -> Result<(Self, Connection<RawCondvar, M, E>), EspError>
    where
        Self: Sized,
    {
        let state = Arc::new(ConnStateGuard::new_default());
        let mut postbox = Postbox::new(state.clone());
        let conn = Connection::new(state.clone());

        let client = unsafe {
            Self::new_nonstatic_generic(url, conf, Some(state), move |event| {
                postbox.post(converter(event))
            })?
        };

        Ok((client, conn))
    }
}

impl EspMqttClient<'static, ()> {
    pub fn new<F>(url: &str, conf: &MqttClientConfiguration, callback: F) -> Result<Self, EspError>
    where
        F: for<'b> FnMut(&'b Result<client::Event<EspMqttMessage<'b>>, EspError>) + Send + 'static,
        Self: Sized,
    {
        Self::new_generic(url, conf, None, callback)
    }
}

impl<'a> EspMqttClient<'a, ()> {
    /// # Safety
    ///
    /// This method - in contrast to method `new` - allows the user to pass
    /// a non-static callback/closure. This enables users to borrow
    /// - in the closure - variables that live on the stack - or more generally - in the same
    /// scope where the service is created.
    ///
    /// HOWEVER: care should be taken NOT to call `core::mem::forget()` on the service,
    /// as that would immediately lead to an UB (crash).
    /// Also note that forgetting the service might happen with `Rc` and `Arc`
    /// when circular references are introduced: https://github.com/rust-lang/rust/issues/24456
    ///
    /// The reason is that the closure is actually sent to a hidden ESP IDF thread.
    /// This means that if the service is forgotten, Rust is free to e.g. unwind the stack
    /// and the closure now owned by this other thread will end up with references to variables that no longer exist.
    ///
    /// The destructor of the service takes care - prior to the service being dropped and e.g.
    /// the stack being unwind - to remove the closure from the hidden thread and destroy it.
    /// Unfortunately, when the service is forgotten, the un-subscription does not happen
    /// and invalid references are left dangling.
    ///
    /// This "local borrowing" will only be possible to express in a safe way once/if `!Leak` types
    /// are introduced to Rust (i.e. the impossibility to "forget" a type and thus not call its destructor).
    pub unsafe fn new_nonstatic<F>(
        url: &str,
        conf: &MqttClientConfiguration,
        callback: F,
    ) -> Result<Self, EspError>
    where
        F: for<'b> FnMut(&'b Result<client::Event<EspMqttMessage<'b>>, EspError>) + Send + 'a,
        Self: Sized,
    {
        Self::new_nonstatic_generic(url, conf, None, callback)
    }
}

impl<S> EspMqttClient<'static, S> {
    pub fn new_generic<F>(
        url: &str,
        conf: &MqttClientConfiguration,
        conn_state_guard: Option<Arc<ConnStateGuard<RawCondvar, S>>>,
        mut callback: F,
    ) -> Result<Self, EspError>
    where
        F: for<'b> FnMut(&'b Result<client::Event<EspMqttMessage<'b>>, EspError>) + Send + 'static,
        Self: Sized,
    {
        Self::new_raw(
            url,
            conf,
            Box::new(move |event_handle| {
                callback(&EspMqttMessage::new_event(
                    unsafe { event_handle.as_ref() }.unwrap(),
                ));
            }),
            conn_state_guard,
        )
    }
}

impl<'a, S> EspMqttClient<'a, S> {
    /// # Safety
    ///
    /// This method - in contrast to method `new_generic` - allows the user to pass
    /// a non-static callback/closure. This enables users to borrow
    /// - in the closure - variables that live on the stack - or more generally - in the same
    /// scope where the service is created.
    ///
    /// HOWEVER: care should be taken NOT to call `core::mem::forget()` on the service,
    /// as that would immediately lead to an UB (crash).
    /// Also note that forgetting the service might happen with `Rc` and `Arc`
    /// when circular references are introduced: https://github.com/rust-lang/rust/issues/24456
    ///
    /// The reason is that the closure is actually sent to a hidden ESP IDF thread.
    /// This means that if the service is forgotten, Rust is free to e.g. unwind the stack
    /// and the closure now owned by this other thread will end up with references to variables that no longer exist.
    ///
    /// The destructor of the service takes care - prior to the service being dropped and e.g.
    /// the stack being unwind - to remove the closure from the hidden thread and destroy it.
    /// Unfortunately, when the service is forgotten, the un-subscription does not happen
    /// and invalid references are left dangling.
    ///
    /// This "local borrowing" will only be possible to express in a safe way once/if `!Leak` types
    /// are introduced to Rust (i.e. the impossibility to "forget" a type and thus not call its destructor).
    pub unsafe fn new_nonstatic_generic<F>(
        url: &str,
        conf: &MqttClientConfiguration,
        conn_state_guard: Option<Arc<ConnStateGuard<RawCondvar, S>>>,
        mut callback: F,
    ) -> Result<Self, EspError>
    where
        F: for<'b> FnMut(&'b Result<client::Event<EspMqttMessage<'b>>, EspError>) + Send + 'a,
        Self: Sized,
    {
        Self::new_raw(
            url,
            conf,
            Box::new(move |event_handle| {
                callback(&EspMqttMessage::new_event(
                    unsafe { event_handle.as_ref() }.unwrap(),
                ));
            }),
            conn_state_guard,
        )
    }

    fn new_raw(
        url: &str,
        conf: &MqttClientConfiguration,
        raw_callback: Box<dyn FnMut(esp_mqtt_event_handle_t) + Send + 'a>,
        conn_state_guard: Option<Arc<ConnStateGuard<RawCondvar, S>>>,
    ) -> Result<Self, EspError>
    where
        Self: Sized,
    {
        let mut boxed_raw_callback = Box::new(raw_callback);

        let unsafe_callback = UnsafeCallback::from(&mut boxed_raw_callback);

        let (mut c_conf, mut cstrs, tls_psk_conf) = conf.try_into()?;

        #[cfg(esp_idf_version_major = "4")]
        {
            c_conf.uri = cstrs.as_ptr(url)?;
        }

        #[cfg(not(esp_idf_version_major = "4"))]
        {
            c_conf.broker.address.uri = cstrs.as_ptr(url)?;
        }

        #[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
        {
            #[cfg(esp_idf_version_major = "4")]
            if let Some(ref conf) = tls_psk_conf {
                c_conf.psk_hint_key = &*conf.psk;
            }
            #[cfg(not(esp_idf_version_major = "4"))]
            if let Some(ref conf) = tls_psk_conf {
                c_conf.broker.verification.psk_hint_key = &*conf.psk;
            }
        }

        let raw_client = unsafe { esp_mqtt_client_init(&c_conf as *const _) };
        if raw_client.is_null() {
            return Err(EspError::from_infallible::<ESP_FAIL>());
        }

        let client = Self {
            raw_client,
            _boxed_raw_callback: boxed_raw_callback,
            conn_state_guard,
            _tls_psk_conf: tls_psk_conf,
        };

        esp!(unsafe {
            esp_mqtt_client_register_event(
                client.raw_client,
                esp_mqtt_event_id_t_MQTT_EVENT_ANY,
                Some(Self::handle),
                unsafe_callback.as_ptr(),
            )
        })?;

        esp!(unsafe { esp_mqtt_client_start(client.raw_client) })?;

        Ok(client)
    }

    pub fn subscribe(
        &mut self,
        topic: &str,
        qos: client::QoS,
    ) -> Result<client::MessageId, EspError> {
        let c_topic = to_cstring_arg(topic)?;

        #[cfg(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
            all(
                esp_idf_version_major = "5",
                esp_idf_version_minor = "1",
                any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")
            )
        ))]
        let res = Self::check(unsafe {
            esp_mqtt_client_subscribe(self.raw_client, c_topic.as_ptr(), qos as _)
        });

        #[cfg(not(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
            all(
                esp_idf_version_major = "5",
                esp_idf_version_minor = "1",
                any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")
            )
        )))]
        let res = Self::check(unsafe {
            esp_mqtt_client_subscribe_single(self.raw_client, c_topic.as_ptr(), qos as _)
        });

        res
    }

    pub fn unsubscribe(&mut self, topic: &str) -> Result<client::MessageId, EspError> {
        let c_topic = to_cstring_arg(topic)?;

        Self::check(unsafe { esp_mqtt_client_unsubscribe(self.raw_client, c_topic.as_ptr()) })
    }

    pub fn publish(
        &mut self,
        topic: &str,
        qos: client::QoS,
        retain: bool,
        payload: &[u8],
    ) -> Result<client::MessageId, EspError> {
        let c_topic = to_cstring_arg(topic)?;

        let payload_ptr = match payload.len() {
            0 => core::ptr::null(),
            _ => payload.as_ptr(),
        };

        Self::check(unsafe {
            esp_mqtt_client_publish(
                self.raw_client,
                c_topic.as_ptr(),
                payload_ptr as _,
                payload.len() as _,
                qos as _,
                retain as _,
            )
        })
    }

    pub fn enqueue(
        &mut self,
        topic: &str,
        qos: client::QoS,
        retain: bool,
        payload: &[u8],
    ) -> Result<client::MessageId, EspError> {
        let c_topic = to_cstring_arg(topic)?;

        let payload_ptr = match payload.len() {
            0 => core::ptr::null(),
            _ => payload.as_ptr(),
        };

        Self::check(unsafe {
            esp_mqtt_client_enqueue(
                self.raw_client,
                c_topic.as_ptr(),
                payload_ptr as _,
                payload.len() as _,
                qos as _,
                retain as _,
                true,
            )
        })
    }

    extern "C" fn handle(
        event_handler_arg: *mut c_void,
        _event_base: esp_event_base_t,
        _event_id: i32,
        event_data: *mut c_void,
    ) {
        unsafe {
            UnsafeCallback::from_ptr(event_handler_arg).call(event_data as _);
        }
    }

    fn check(result: i32) -> Result<client::MessageId, EspError> {
        match EspError::from(result) {
            Some(err) if result < 0 => Err(err),
            _ => Ok(result as _),
        }
    }
}

impl<'a, P> Drop for EspMqttClient<'a, P> {
    fn drop(&mut self) {
        let connection_state = self.conn_state_guard.take();
        if let Some(connection_state) = connection_state {
            connection_state.close();
        }

        esp!(unsafe { esp_mqtt_client_destroy(self.raw_client) }).unwrap();
    }
}

impl<'a, P> ErrorType for EspMqttClient<'a, P> {
    type Error = EspError;
}

impl<'a, P> client::Client for EspMqttClient<'a, P> {
    fn subscribe(
        &mut self,
        topic: &str,
        qos: client::QoS,
    ) -> Result<client::MessageId, Self::Error> {
        EspMqttClient::subscribe(self, topic, qos)
    }

    fn unsubscribe(&mut self, topic: &str) -> Result<client::MessageId, Self::Error> {
        EspMqttClient::unsubscribe(self, topic)
    }
}

impl<'a, P> client::Publish for EspMqttClient<'a, P> {
    fn publish(
        &mut self,
        topic: &str,
        qos: client::QoS,
        retain: bool,
        payload: &[u8],
    ) -> Result<client::MessageId, Self::Error> {
        EspMqttClient::publish(self, topic, qos, retain, payload)
    }
}

impl<'a, P> client::Enqueue for EspMqttClient<'a, P> {
    fn enqueue(
        &mut self,
        topic: &str,
        qos: client::QoS,
        retain: bool,
        payload: &[u8],
    ) -> Result<client::MessageId, Self::Error> {
        EspMqttClient::enqueue(self, topic, qos, retain, payload)
    }
}

unsafe impl<'a, P> Send for EspMqttClient<'a, P> {}

pub struct EspMqttMessage<'a> {
    event: &'a esp_mqtt_event_t,
    details: client::Details,
}

impl<'a> EspMqttMessage<'a> {
    #[allow(non_upper_case_globals)]
    fn new_event(
        event: &'a esp_mqtt_event_t,
    ) -> Result<client::Event<EspMqttMessage<'a>>, EspError> {
        match event.event_id {
            esp_mqtt_event_id_t_MQTT_EVENT_ERROR => Err(EspError::from_infallible::<ESP_FAIL>()), // TODO
            esp_mqtt_event_id_t_MQTT_EVENT_BEFORE_CONNECT => Ok(client::Event::BeforeConnect),
            esp_mqtt_event_id_t_MQTT_EVENT_CONNECTED => {
                Ok(client::Event::Connected(event.session_present != 0))
            }
            esp_mqtt_event_id_t_MQTT_EVENT_DISCONNECTED => Ok(client::Event::Disconnected),
            esp_mqtt_event_id_t_MQTT_EVENT_SUBSCRIBED => {
                Ok(client::Event::Subscribed(event.msg_id as _))
            }
            esp_mqtt_event_id_t_MQTT_EVENT_UNSUBSCRIBED => {
                Ok(client::Event::Unsubscribed(event.msg_id as _))
            }
            esp_mqtt_event_id_t_MQTT_EVENT_PUBLISHED => {
                Ok(client::Event::Published(event.msg_id as _))
            }
            esp_mqtt_event_id_t_MQTT_EVENT_DATA => {
                Ok(client::Event::Received(EspMqttMessage::new(event)))
            }
            esp_mqtt_event_id_t_MQTT_EVENT_DELETED => Ok(client::Event::Deleted(event.msg_id as _)),
            other => panic!("Unknown message type: {}", other),
        }
    }

    fn new(event: &'a esp_mqtt_event_t) -> Self {
        let mut message = Self {
            event,
            details: client::Details::Complete,
        };

        message.fill_chunk_details();

        message
    }

    pub fn id(&self) -> MessageId {
        self.event.msg_id as _
    }

    pub fn data(&self) -> &[u8] {
        if self.event.data_len > 0 {
            unsafe {
                slice::from_raw_parts(
                    (self.event.data as *const u8).as_ref().unwrap(),
                    self.event.data_len as _,
                )
            }
        } else {
            &[]
        }
    }

    pub fn topic(&self) -> Option<&str> {
        let ptr = self.event.topic;

        if ptr.is_null() {
            None
        } else {
            let len = self.event.topic_len;

            let topic = unsafe {
                let slice = slice::from_raw_parts(ptr as _, len.try_into().unwrap());
                core::str::from_utf8(slice).unwrap()
            };

            Some(topic)
        }
    }

    pub fn details(&self) -> &Details {
        &self.details
    }

    fn fill_chunk_details(&mut self) {
        if self.event.data_len < self.event.total_data_len {
            if self.event.current_data_offset == 0 {
                self.details = client::Details::InitialChunk(client::InitialChunkData {
                    total_data_size: self.event.total_data_len as _,
                });
            } else {
                self.details = client::Details::SubsequentChunk(client::SubsequentChunkData {
                    current_data_offset: self.event.current_data_offset as _,
                    total_data_size: self.event.total_data_len as _,
                });
            }
        }
    }
}

impl<'a> Debug for EspMqttMessage<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[id = {}, topic = {:?}, details = {:?}]",
            self.id(),
            self.topic(),
            self.details()
        )
    }
}

impl<'a> client::Message for EspMqttMessage<'a> {
    fn id(&self) -> client::MessageId {
        EspMqttMessage::id(self)
    }

    fn data(&self) -> &[u8] {
        EspMqttMessage::data(self)
    }

    fn topic(&self) -> Option<&str> {
        EspMqttMessage::topic(self)
    }

    fn details(&self) -> &client::Details {
        EspMqttMessage::details(self)
    }
}

mod asyncify {
    use core::fmt::Debug;

    extern crate alloc;

    use alloc::sync::Arc;

    use embedded_svc::mqtt::client::{self, MessageImpl};
    use embedded_svc::utils::asyncify::mqtt::client::{
        AsyncClient, AsyncConnState, AsyncConnection, AsyncPostbox, Blocking, Publishing,
    };
    use embedded_svc::utils::asyncify::{Asyncify, UnblockingAsyncify};
    use embedded_svc::utils::mqtt::client::ConnStateGuard;
    use embedded_svc::utils::mutex::Mutex;

    use crate::sys::EspError;

    use crate::private::mutex::{RawCondvar, RawMutex};

    use super::{EspMqttClient, EspMqttMessage, MqttClientConfiguration};

    impl<'a, P> UnblockingAsyncify for super::EspMqttClient<'a, P> {
        type AsyncWrapper<U, S> = AsyncClient<U, Arc<Mutex<RawMutex, S>>>;
    }

    impl<'a, P> Asyncify for super::EspMqttClient<'a, P> {
        type AsyncWrapper<S> = AsyncClient<(), Blocking<S, Publishing>>;
    }

    pub type EspMqttAsyncClient<'a> = EspMqttConvertingAsyncClient<'a, MessageImpl, EspError>;

    pub type EspMqttUnblockingAsyncClient<'a, U> =
        EspMqttConvertingUnblockingAsyncClient<'a, U, MessageImpl, EspError>;

    pub type EspMqttAsyncConnection = EspMqttConvertingAsyncConnection<MessageImpl, EspError>;

    pub type EspMqttConvertingUnblockingAsyncClient<'a, U, M, E> =
        AsyncClient<U, Arc<Mutex<RawMutex, EspMqttClient<'a, AsyncConnState<M, E>>>>>;

    pub type EspMqttConvertingAsyncClient<'a, M, E> =
        AsyncClient<(), EspMqttClient<'a, AsyncConnState<M, E>>>;

    pub type EspMqttConvertingAsyncConnection<M, E> = AsyncConnection<RawCondvar, M, E>;

    impl EspMqttClient<'static, AsyncConnState<MessageImpl, EspError>> {
        pub fn new_with_async_conn(
            url: &str,
            conf: &MqttClientConfiguration,
        ) -> Result<(Self, EspMqttAsyncConnection), EspError>
        where
            Self: Sized,
        {
            Self::new_with_converting_async_conn(url, conf, move |r| {
                r.as_ref()
                    .map(|event| event.transform_received(MessageImpl::new))
                    .map_err(|e| *e)
            })
        }
    }

    impl<M, E> EspMqttClient<'static, AsyncConnState<M, E>>
    where
        M: Send + 'static,
        E: Debug + Send + 'static,
    {
        pub fn new_with_converting_async_conn<F>(
            url: &str,
            conf: &MqttClientConfiguration,
            converter: F,
        ) -> Result<(Self, EspMqttConvertingAsyncConnection<M, E>), EspError>
        where
            F: for<'b> FnMut(
                    &'b Result<client::Event<EspMqttMessage<'b>>, EspError>,
                ) -> Result<client::Event<M>, E>
                + Send
                + 'static,
            Self: Sized,
        {
            Self::internal_new_with_converting_async_conn(url, conf, converter)
        }
    }

    impl<'a, M, E> EspMqttClient<'a, AsyncConnState<M, E>>
    where
        M: Send + 'static,
        E: Debug + Send + 'static,
    {
        /// # Safety
        ///
        /// This method - in contrast to method `new_with_converting_async_conn` - allows the user to pass
        /// a non-static callback/closure. This enables users to borrow
        /// - in the closure - variables that live on the stack - or more generally - in the same
        /// scope where the service is created.
        ///
        /// HOWEVER: care should be taken NOT to call `core::mem::forget()` on the service,
        /// as that would immediately lead to an UB (crash).
        /// Also note that forgetting the service might happen with `Rc` and `Arc`
        /// when circular references are introduced: https://github.com/rust-lang/rust/issues/24456
        ///
        /// The reason is that the closure is actually sent to a hidden ESP IDF thread.
        /// This means that if the service is forgotten, Rust is free to e.g. unwind the stack
        /// and the closure now owned by this other thread will end up with references to variables that no longer exist.
        ///
        /// The destructor of the service takes care - prior to the service being dropped and e.g.
        /// the stack being unwind - to remove the closure from the hidden thread and destroy it.
        /// Unfortunately, when the service is forgotten, the un-subscription does not happen
        /// and invalid references are left dangling.
        ///
        /// This "local borrowing" will only be possible to express in a safe way once/if `!Leak` types
        /// are introduced to Rust (i.e. the impossibility to "forget" a type and thus not call its destructor).
        pub unsafe fn new_nonstatic_with_converting_async_conn<F>(
            url: &str,
            conf: &MqttClientConfiguration,
            converter: F,
        ) -> Result<(Self, EspMqttConvertingAsyncConnection<M, E>), EspError>
        where
            F: for<'b> FnMut(
                    &'b Result<client::Event<EspMqttMessage<'b>>, EspError>,
                ) -> Result<client::Event<M>, E>
                + Send
                + 'a,
            Self: Sized,
        {
            Self::internal_new_with_converting_async_conn(url, conf, converter)
        }

        fn internal_new_with_converting_async_conn(
            url: &str,
            conf: &MqttClientConfiguration,
            mut converter: impl for<'b> FnMut(
                    &'b Result<client::Event<EspMqttMessage<'b>>, EspError>,
                ) -> Result<client::Event<M>, E>
                + Send
                + 'a,
        ) -> Result<(Self, EspMqttConvertingAsyncConnection<M, E>), EspError>
        where
            Self: Sized,
        {
            let state = Arc::new(ConnStateGuard::new_default());
            let mut postbox = AsyncPostbox::new(state.clone());
            let conn = AsyncConnection::new(state.clone());

            let client = unsafe {
                Self::new_nonstatic_generic(url, conf, Some(state), move |event| {
                    postbox.post(converter(event))
                })?
            };

            Ok((client, conn))
        }
    }
}
