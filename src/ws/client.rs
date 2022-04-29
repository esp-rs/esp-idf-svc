use alloc::sync::Arc;
use core::{
    convert::{TryFrom, TryInto},
    time,
};

use embedded_svc::{
    errors::Errors,
    ws::{FrameType, Sender},
};
use esp_idf_hal::{
    delay::TickType,
    mutex::{Condvar, Mutex},
};
use esp_idf_sys::{
    c_types, esp, esp_event_base_t, esp_websocket_client_close, esp_websocket_client_config_t,
    esp_websocket_client_destroy, esp_websocket_client_handle_t, esp_websocket_client_init,
    esp_websocket_client_send_bin, esp_websocket_client_send_text, esp_websocket_client_start,
    esp_websocket_event_data_t, esp_websocket_event_id_t_WEBSOCKET_EVENT_ANY,
    esp_websocket_event_id_t_WEBSOCKET_EVENT_CLOSED,
    esp_websocket_event_id_t_WEBSOCKET_EVENT_CONNECTED,
    esp_websocket_event_id_t_WEBSOCKET_EVENT_DATA,
    esp_websocket_event_id_t_WEBSOCKET_EVENT_DISCONNECTED,
    esp_websocket_event_id_t_WEBSOCKET_EVENT_ERROR, esp_websocket_event_id_t_WEBSOCKET_EVENT_MAX,
    esp_websocket_register_events, esp_websocket_transport_t,
    esp_websocket_transport_t_WEBSOCKET_TRANSPORT_OVER_SSL,
    esp_websocket_transport_t_WEBSOCKET_TRANSPORT_OVER_TCP,
    esp_websocket_transport_t_WEBSOCKET_TRANSPORT_UNKNOWN, ifreq, EspError, TickType_t, ESP_FAIL,
};

use crate::private::common::Newtype;
use crate::private::cstr::{self, RawCstrs};

use ::anyhow::bail;

pub enum EspWebSocketTransport {
    TransportUnknown,
    TransportOverTCP,
    TransportOverSSL,
}

impl Default for EspWebSocketTransport {
    fn default() -> Self {
        Self::TransportUnknown
    }
}

impl From<EspWebSocketTransport> for Newtype<esp_websocket_transport_t> {
    fn from(transport: EspWebSocketTransport) -> Self {
        Newtype(match transport {
            EspWebSocketTransport::TransportUnknown => {
                esp_websocket_transport_t_WEBSOCKET_TRANSPORT_UNKNOWN
            }
            EspWebSocketTransport::TransportOverTCP => {
                esp_websocket_transport_t_WEBSOCKET_TRANSPORT_OVER_TCP
            }
            EspWebSocketTransport::TransportOverSSL => {
                esp_websocket_transport_t_WEBSOCKET_TRANSPORT_OVER_SSL
            }
        })
    }
}

pub struct WebSocketEvent<'a> {
    pub event_type: WebSocketEventType<'a>,
    state: Arc<EspWebSocketConnectionState>,
}

impl<'a> WebSocketEvent<'a> {
    fn new(
        event_id: i32,
        event_data: &'a esp_websocket_event_data_t,
        state: &Arc<EspWebSocketConnectionState>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            event_type: WebSocketEventType::new(event_id, event_data)?,
            state: state.clone(),
        })
    }
}

impl<'a> Drop for WebSocketEvent<'a> {
    fn drop(&mut self) {
        let mut message = self.state.message.lock();

        if message.take().is_some() {
            self.state.processed.notify_all();
        }
    }
}

#[derive(Debug)]
pub enum WebSocketClosingReason {
    PurposeFulfilled,
    GoingAway,
    ProtocolError,
    UnacceptableData,
    Reserved(u16),
    InconsistentType,
    PolicyViolated,
    MessageTooBig,
    ExtensionNotReturned,
    UnexpectedCondition,
}

impl WebSocketClosingReason {
    fn new(code: u16) -> anyhow::Result<Self> {
        match code {
            1000 => Ok(Self::PurposeFulfilled),
            1001 => Ok(Self::GoingAway),
            1002 => Ok(Self::ProtocolError),
            1003 => Ok(Self::UnacceptableData),
            1004..=1006 | 1015 => Ok(Self::Reserved(code)),
            1007 => Ok(Self::InconsistentType),
            1008 => Ok(Self::PolicyViolated),
            1009 => Ok(Self::MessageTooBig),
            1010 => Ok(Self::ExtensionNotReturned),
            1011 => Ok(Self::UnexpectedCondition),
            _ => bail!("Unsupported closing status code"),
        }
    }
}

#[derive(Debug)]
pub enum WebSocketEventType<'a> {
    Connected,
    Disconnected,
    Close(Option<WebSocketClosingReason>),
    Closed,
    Text(&'a str),
    Binary(&'a [u8]),
}

impl<'a> WebSocketEventType<'a> {
    pub fn new(event_id: i32, event_data: &'a esp_websocket_event_data_t) -> anyhow::Result<Self> {
        #[allow(non_upper_case_globals)]
        match event_id {
            esp_websocket_event_id_t_WEBSOCKET_EVENT_ERROR => bail!("WebSocket Event Error"),
            esp_websocket_event_id_t_WEBSOCKET_EVENT_CONNECTED => Ok(Self::Connected),
            esp_websocket_event_id_t_WEBSOCKET_EVENT_DISCONNECTED => Ok(Self::Disconnected),
            esp_websocket_event_id_t_WEBSOCKET_EVENT_DATA => {
                match event_data.op_code {
                    // Text frame
                    1 => Ok(Self::Text({
                        unsafe {
                            let slice = core::slice::from_raw_parts(
                                event_data.data_ptr as *const u8,
                                event_data.data_len as usize,
                            );
                            core::str::from_utf8(slice)?
                        }
                    })),
                    // Binary frame
                    2 => Ok(Self::Binary(unsafe {
                        core::slice::from_raw_parts(
                            event_data.data_ptr as *const u8,
                            event_data.data_len as usize,
                        )
                    })),
                    // Closing Frame
                    // may contain a reason for closing the connection
                    8 => Ok(Self::Close(if event_data.data_len >= 2 {
                        Some(WebSocketClosingReason::new(u16::from_be(
                            event_data.data_ptr as _,
                        ))?)
                    } else {
                        None
                    })),
                    _ => bail!(
                        "WebSocket Data Event contains wrong OpCode: {}",
                        event_data.op_code
                    ),
                }
            }
            esp_websocket_event_id_t_WEBSOCKET_EVENT_CLOSED => Ok(Self::Closed),
            esp_websocket_event_id_t_WEBSOCKET_EVENT_MAX => {
                // TODO: is this the correct meaning of EVENT_MAX?
                bail!("Exceeded maximum packet size")
            }
            default => bail!("Unrecognized WebSocket event: {}", default),
        }
    }
}

#[derive(Default)]
pub struct EspWebSocketClientConfig<'a> {
    pub uri: Option<&'a str>,
    pub host: Option<&'a str>,
    pub port: u16,
    pub username: Option<&'a str>,
    pub password: Option<&'a str>,
    pub path: Option<&'a str>,
    pub disable_auto_reconnect: bool,
    // TODO: pub user_context:
    pub task_prio: u8,
    pub task_stack: u8,
    pub buffer_size: usize,
    pub transport: EspWebSocketTransport,
    pub subprotocol: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub headers: Option<&'a str>,
    pub pingpong_timeout_sec: time::Duration,
    pub disable_pingpong_discon: bool,
    pub use_global_ca_store: bool,
    pub skip_cert_common_name_check: bool,
    pub keep_alive_idle: Option<time::Duration>,
    pub keep_alive_interval: Option<time::Duration>,
    pub keep_alive_count: Option<u16>,
    pub reconnect_timeout_ms: time::Duration,
    pub network_timeout_ms: time::Duration,
    pub ping_interval_sec: time::Duration,
    pub if_name: Option<&'a str>,
    pub cert_pem: Option<&'a str>,
    pub client_cert: Option<&'a str>,
    pub client_key: Option<&'a str>,
}

impl<'a> TryFrom<EspWebSocketClientConfig<'a>> for (esp_websocket_client_config_t, RawCstrs) {
    type Error = anyhow::Error;

    fn try_from(conf: EspWebSocketClientConfig) -> Result<Self, Self::Error> {
        let mut cstrs = RawCstrs::new();

        let mut c_conf = esp_websocket_client_config_t {
            uri: cstrs.as_nptr(conf.uri),
            host: cstrs.as_nptr(conf.host),
            port: conf.port.into(),
            username: cstrs.as_nptr(conf.username),
            password: cstrs.as_nptr(conf.password),
            path: cstrs.as_nptr(conf.path),
            disable_auto_reconnect: conf.disable_auto_reconnect,
            // TODO user_context: *mut c_types::c_void,
            user_context: core::ptr::null_mut(),

            task_prio: conf.task_prio as _,
            task_stack: conf.task_stack as _,
            buffer_size: conf.buffer_size as _,

            transport: Newtype::<esp_websocket_transport_t>::from(conf.transport).0,

            subprotocol: cstrs.as_nptr(conf.subprotocol),
            user_agent: cstrs.as_nptr(conf.user_agent),
            headers: cstrs.as_nptr(conf.headers),

            pingpong_timeout_sec: conf.pingpong_timeout_sec.as_secs() as _,
            disable_pingpong_discon: conf.disable_pingpong_discon,

            use_global_ca_store: conf.use_global_ca_store,
            skip_cert_common_name_check: conf.skip_cert_common_name_check,

            ping_interval_sec: conf.ping_interval_sec.as_secs() as _,

            cert_pem: cstrs.as_nptr(conf.cert_pem),
            cert_len: conf.cert_pem.map(|c| c.len()).unwrap_or(0) as _,
            client_cert: cstrs.as_nptr(conf.client_cert),
            client_cert_len: conf.client_cert.map(|c| c.len()).unwrap_or(0) as _,
            client_key: cstrs.as_nptr(conf.client_key),
            client_key_len: conf.client_key.map(|c| c.len()).unwrap_or(0) as _,

            // NOTE: default keep_alive_* values are set below, so they are not explicitly listed
            // here
            // some validation has to be done on if_name, so it is set to a default value first
            // before overwriting it later after the validation
            // to compile, the values are being set to a default value first before possibly
            // overwriting them
            ..Default::default()
        };

        if let Some(if_name) = conf.if_name {
            if !(if_name.len() == 6 && if_name.is_ascii()) {
                bail!("if_name needs to be a 6 character long ASCII string");
            }
            let mut s: [c_types::c_char; 6] = [c_types::c_char::default(); 6];
            for (i, c) in if_name.chars().enumerate() {
                s[i] = c as _;
            }

            let mut ifreq = ifreq { ifr_name: s };
            c_conf.if_name = &mut ifreq as *mut ifreq;
        }

        if let Some(idle) = conf.keep_alive_idle {
            c_conf.keep_alive_enable = true;
            c_conf.keep_alive_idle = idle.as_secs() as _;
        }

        if let Some(interval) = conf.keep_alive_interval {
            c_conf.keep_alive_enable = true;
            c_conf.keep_alive_interval = interval.as_secs() as _;
        }

        if let Some(count) = conf.keep_alive_count {
            c_conf.keep_alive_enable = true;
            c_conf.keep_alive_count = count.into();
        }

        if let Some(keep_alive_idle) = conf.keep_alive_idle {
            c_conf.keep_alive_enable = true;
            c_conf.keep_alive_idle = keep_alive_idle.as_secs() as _;
        }

        Ok((c_conf, cstrs))
    }
}

struct UnsafeCallback(*mut Box<dyn FnMut(i32, *mut esp_websocket_event_data_t)>);

impl UnsafeCallback {
    fn from(boxed: &mut Box<Box<dyn FnMut(i32, *mut esp_websocket_event_data_t)>>) -> Self {
        Self(boxed.as_mut())
    }

    unsafe fn from_ptr(ptr: *mut c_types::c_void) -> Self {
        Self(ptr as *mut _)
    }

    fn as_ptr(&self) -> *mut c_types::c_void {
        self.0 as *mut _
    }

    unsafe fn call(&self, event_id: i32, data: *mut esp_websocket_event_data_t) {
        let reference = self.0.as_mut().unwrap();

        (reference)(event_id, data);
    }
}

#[cfg_attr(version("1.61"), allow(suspicious_auto_trait_impls))]
unsafe impl Send for Newtype<*mut esp_websocket_event_data_t> {}

struct EspWebSocketConnectionState {
    message: Mutex<Option<(i32, Newtype<*mut esp_websocket_event_data_t>)>>,
    posted: Condvar,
    processed: Condvar,
}

impl Default for EspWebSocketConnectionState {
    fn default() -> Self {
        Self {
            message: Mutex::new(None),
            posted: Condvar::new(),
            processed: Condvar::new(),
        }
    }
}

#[derive(Clone)]
pub struct EspWebSocketConnection(Arc<EspWebSocketConnectionState>);

impl Default for EspWebSocketConnection {
    fn default() -> Self {
        Self(Arc::new(EspWebSocketConnectionState::default()))
    }
}

impl EspWebSocketConnection {
    fn post(&self, event_id: i32, event: *mut esp_websocket_event_data_t) {
        let mut message = self.0.message.lock();

        *message = Some((event_id, Newtype(event)));
        self.0.posted.notify_all();

        while message.is_some() {
            message = self.0.processed.wait(message);
        }
    }

    // NOTE: cannot implement the `Iterator` trait as it requires that all the items can be alive
    // at the same time, which is not given here
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<anyhow::Result<WebSocketEvent>> {
        let mut message = self.0.message.lock();

        // wait for new message to arrive
        while message.is_none() {
            message = self.0.posted.wait(message);
        }

        let event_id = message.as_ref().unwrap().0;
        let event = unsafe { message.as_ref().unwrap().1 .0.as_ref() };
        if let Some(event) = event {
            let wse = WebSocketEvent::new(event_id, event, &self.0);

            Some(wse)
        } else {
            None
        }
    }
}

pub struct EspWebSocketClient {
    handle: esp_websocket_client_handle_t,
    // used for the timeout in every call to a send method in the c lib as the
    // `send` method in the `Sender` trait in embedded_svc::ws does not take a timeout itself
    timeout: TickType_t,
    callback: Box<dyn FnMut(i32, *mut esp_websocket_event_data_t)>,
}

impl EspWebSocketClient {
    pub fn new(
        config: EspWebSocketClientConfig,
        timeout: time::Duration,
    ) -> Result<(Self, EspWebSocketConnection), anyhow::Error> {
        let connection = EspWebSocketConnection::default();
        let client_connection = connection.clone();

        let client = Self::new_with_raw_callback(
            config,
            timeout,
            Box::new(move |event_id, event_handle| {
                EspWebSocketConnection::post(&client_connection, event_id, event_handle);
            }),
        )?;

        Ok((client, connection))
    }

    fn new_with_raw_callback(
        config: EspWebSocketClientConfig,
        timeout: time::Duration,
        raw_callback: Box<dyn FnMut(i32, *mut esp_websocket_event_data_t)>,
    ) -> Result<Self, anyhow::Error> {
        let mut boxed_raw_callback = Box::new(raw_callback);
        let unsafe_callback = UnsafeCallback::from(&mut boxed_raw_callback);

        let t: TickType = timeout.into();

        let (conf, _cstrs): (esp_websocket_client_config_t, RawCstrs) = config.try_into()?;
        let handle = unsafe { esp_websocket_client_init(&conf) };

        if handle.is_null() {
            esp!(ESP_FAIL)?;
        }

        let client = Self {
            handle,
            timeout: t.0,
            callback: boxed_raw_callback,
        };

        esp!(unsafe {
            esp_websocket_register_events(
                client.handle,
                esp_websocket_event_id_t_WEBSOCKET_EVENT_ANY,
                Some(Self::handle),
                unsafe_callback.as_ptr(),
            )
        })?;

        esp!(unsafe { esp_websocket_client_start(handle) })?;

        Ok(client)
    }

    extern "C" fn handle(
        event_handler_arg: *mut c_types::c_void,
        _event_base: esp_event_base_t,
        event_id: i32,
        event_data: *mut c_types::c_void,
    ) {
        unsafe {
            UnsafeCallback::from_ptr(event_handler_arg).call(event_id, event_data as _);
        }
    }

    fn check(result: c_types::c_int) -> Result<usize, EspError> {
        if result < 0 {
            esp!(result)?;
        }

        Ok(result as _)
    }

    fn send_data(
        &mut self,
        frame_type: FrameType,
        frame_data: Option<&[u8]>,
    ) -> Result<usize, <EspWebSocketClient as Errors>::Error> {
        let mut content = core::ptr::null();
        let mut content_length: usize = 0;

        if let Some(data) = frame_data {
            content = data.as_ref().as_ptr();
            content_length = data.as_ref().len();
        }

        Self::check(match frame_type {
            FrameType::Binary(false) => unsafe {
                esp_websocket_client_send_bin(
                    self.handle,
                    content as _,
                    content_length as _,
                    self.timeout,
                )
            },
            FrameType::Text(false) => unsafe {
                esp_websocket_client_send_text(
                    self.handle,
                    content as _,
                    content_length as _,
                    self.timeout,
                )
            },
            _ => {
                panic!("Unsupported sending operation!");
            }
        })
    }
}

impl Drop for EspWebSocketClient {
    fn drop(&mut self) {
        esp!(unsafe { esp_websocket_client_close(self.handle, self.timeout) }).unwrap();
        esp!(unsafe { esp_websocket_client_destroy(self.handle) }).unwrap();

        // timeout and callback dropped automatically
    }
}

impl Errors for EspWebSocketClient {
    type Error = EspError;
}

impl Sender for EspWebSocketClient {
    fn send(
        &mut self,
        frame_type: FrameType,
        frame_data: Option<&[u8]>,
    ) -> Result<(), Self::Error> {
        // NOTE: fragmented sending, as well as Closing or Continuing a connection, is not
        // supported by the underlying C library and/or happen implicitly, e.g. when the
        // `EspWebSocketClient` is dropped.
        match frame_type {
            FrameType::Binary(false) | FrameType::Text(false) => {
                self.send_data(frame_type, frame_data)?
            }
            FrameType::Binary(true) | FrameType::Text(true) => {
                panic!("Unsupported operation: Sending of fragmented data!")
            }
            FrameType::Ping | FrameType::Pong => {
                panic!("Unsupported operation: Sending of Ping/Pong frames!")
            }
            FrameType::Close => {
                panic!("Unsupported operation: Closing a connection manually (drop the client instead)!")
            }
            FrameType::SocketClose => {
                panic!("Unsupported operation: Closing a connection manually (drop the client instead)!")
            }
            FrameType::Continue(_) => panic!("Unsupported operation: Sending of fragmented data!"),
        };

        Ok(())
    }
}
