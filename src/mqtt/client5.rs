use std::string::ToString;
use std::vec::Vec;

#[allow(unused_imports)]
pub use super::*;

extern crate alloc;
use alloc::ffi::CString;

use embedded_svc::mqtt::client5::{EventProperty, UserPropertyItem, UserPropertyList};
#[allow(unused_imports)]
use esp_idf_hal::sys::*;

pub struct EspEventProperty {}

pub struct EspUserPropertyList(pub(crate) mqtt5_user_property_handle_t);

/// MQTT5 protocol error reason codes as defined in MQTT5 protocol document section 2.4
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u32)]
pub enum ErrorReasonCode {
    /// Unspecified error
    UnspecifiedError = 0x80,
    /// The received packet does not conform to this specification
    MalformedPacket = 0x81,
    /// An unexpected or out of order packet was received
    ProtocolError = 0x82,
    /// Implementation specific error
    ImplementSpecificError = 0x83,
    /// The server does not support the level of the MQTT protocol requested by the client
    UnsupportedProtocolVersion = 0x84,
    /// The client identifier is not valid
    InvalidClientId = 0x85,
    /// The server does not accept the user name or password specified by the client
    BadUsernameOrPassword = 0x86,
    /// The client is not authorized to connect
    NotAuthorized = 0x87,
    /// The MQTT server is not available
    ServerUnavailable = 0x88,
    /// The server is busy. Try again later
    ServerBusy = 0x89,
    /// This client has been banned by administrative action
    Banned = 0x8A,
    /// The server is shutting down
    ServerShuttingDown = 0x8B,
    /// The authentication method is not supported
    BadAuthMethod = 0x8C,
    /// The connection is closed because no packet has been received for 1.5 times the keep alive time
    KeepAliveTimeout = 0x8D,
    /// Another connection using the same client ID has connected
    SessionTakenOver = 0x8E,
    /// The topic filter is not valid
    TopicFilterInvalid = 0x8F,
    /// The topic name is not valid
    TopicNameInvalid = 0x90,
    /// The packet identifier is already in use
    PacketIdentifierInUse = 0x91,
    /// The packet identifier is not found
    PacketIdentifierNotFound = 0x92,
    /// The client has received more than receive maximum publication
    ReceiveMaximumExceeded = 0x93,
    /// The topic alias is not valid
    TopicAliasInvalid = 0x94,
    /// The packet exceeded the maximum permissible size
    PacketTooLarge = 0x95,
    /// The message rate is too high
    MessageRateTooHigh = 0x96,
    /// An implementation or administrative imposed limit has been exceeded
    QuotaExceeded = 0x97,
    /// The connection is closed due to an administrative action
    AdministrativeAction = 0x98,
    /// The payload format does not match the specified format indicator
    PayloadFormatInvalid = 0x99,
    /// The server does not support retained messages
    RetainNotSupported = 0x9A,
    /// The server does not support the QoS requested
    QosNotSupported = 0x9B,
    /// The client should temporarily use another server
    UseAnotherServer = 0x9C,
    /// The server has moved and the client should permanently use another server
    ServerMoved = 0x9D,
    /// The server does not support shared subscriptions
    SharedSubscriptionNotSupported = 0x9E,
    /// The connection rate limit has been exceeded
    ConnectionRateExceeded = 0x9F,
    /// The maximum connection time authorized has been exceeded
    MaximumConnectTime = 0xA0,
    /// The server does not support subscription identifiers
    SubscribeIdentifierNotSupported = 0xA1,
    /// The server does not support wildcard subscriptions
    WildcardSubscriptionNotSupported = 0xA2,
}

impl From<ErrorReasonCode> for mqtt5_error_reason_code {
    fn from(code: ErrorReasonCode) -> Self {
        code as u32
    }
}

#[allow(non_upper_case_globals)]
impl From<mqtt5_error_reason_code> for ErrorReasonCode {
    fn from(code: mqtt5_error_reason_code) -> Self {
        match code {
            mqtt5_error_reason_code_MQTT5_UNSPECIFIED_ERROR => ErrorReasonCode::UnspecifiedError,
            mqtt5_error_reason_code_MQTT5_MALFORMED_PACKET => ErrorReasonCode::MalformedPacket,
            mqtt5_error_reason_code_MQTT5_PROTOCOL_ERROR => ErrorReasonCode::ProtocolError,
            mqtt5_error_reason_code_MQTT5_IMPLEMENT_SPECIFIC_ERROR => ErrorReasonCode::ImplementSpecificError,
            mqtt5_error_reason_code_MQTT5_UNSUPPORTED_PROTOCOL_VER => ErrorReasonCode::UnsupportedProtocolVersion,
            mqtt5_error_reason_code_MQTT5_INVALID_CLIENT_ID => ErrorReasonCode::InvalidClientId,
            mqtt5_error_reason_code_MQTT5_BAD_USERNAME_OR_PWD => ErrorReasonCode::BadUsernameOrPassword,
            mqtt5_error_reason_code_MQTT5_NOT_AUTHORIZED => ErrorReasonCode::NotAuthorized,
            mqtt5_error_reason_code_MQTT5_SERVER_UNAVAILABLE => ErrorReasonCode::ServerUnavailable,
            mqtt5_error_reason_code_MQTT5_SERVER_BUSY => ErrorReasonCode::ServerBusy,
            mqtt5_error_reason_code_MQTT5_BANNED => ErrorReasonCode::Banned,
            mqtt5_error_reason_code_MQTT5_SERVER_SHUTTING_DOWN => ErrorReasonCode::ServerShuttingDown,
            mqtt5_error_reason_code_MQTT5_BAD_AUTH_METHOD => ErrorReasonCode::BadAuthMethod,
            mqtt5_error_reason_code_MQTT5_KEEP_ALIVE_TIMEOUT => ErrorReasonCode::KeepAliveTimeout,
            mqtt5_error_reason_code_MQTT5_SESSION_TAKEN_OVER => ErrorReasonCode::SessionTakenOver,
            mqtt5_error_reason_code_MQTT5_TOPIC_FILTER_INVALID => ErrorReasonCode::TopicFilterInvalid,
            mqtt5_error_reason_code_MQTT5_TOPIC_NAME_INVALID => ErrorReasonCode::TopicNameInvalid,
            mqtt5_error_reason_code_MQTT5_PACKET_IDENTIFIER_IN_USE => ErrorReasonCode::PacketIdentifierInUse,
            mqtt5_error_reason_code_MQTT5_PACKET_IDENTIFIER_NOT_FOUND => ErrorReasonCode::PacketIdentifierNotFound,
            mqtt5_error_reason_code_MQTT5_RECEIVE_MAXIMUM_EXCEEDED => ErrorReasonCode::ReceiveMaximumExceeded,
            mqtt5_error_reason_code_MQTT5_TOPIC_ALIAS_INVALID => ErrorReasonCode::TopicAliasInvalid,
            mqtt5_error_reason_code_MQTT5_PACKET_TOO_LARGE => ErrorReasonCode::PacketTooLarge,
            mqtt5_error_reason_code_MQTT5_MESSAGE_RATE_TOO_HIGH => ErrorReasonCode::MessageRateTooHigh,
            mqtt5_error_reason_code_MQTT5_QUOTA_EXCEEDED => ErrorReasonCode::QuotaExceeded,
            mqtt5_error_reason_code_MQTT5_ADMINISTRATIVE_ACTION => ErrorReasonCode::AdministrativeAction,
            mqtt5_error_reason_code_MQTT5_PAYLOAD_FORMAT_INVALID => ErrorReasonCode::PayloadFormatInvalid,
            mqtt5_error_reason_code_MQTT5_RETAIN_NOT_SUPPORT => ErrorReasonCode::RetainNotSupported,
            mqtt5_error_reason_code_MQTT5_QOS_NOT_SUPPORT => ErrorReasonCode::QosNotSupported,
            mqtt5_error_reason_code_MQTT5_USE_ANOTHER_SERVER => ErrorReasonCode::UseAnotherServer,
            mqtt5_error_reason_code_MQTT5_SERVER_MOVED => ErrorReasonCode::ServerMoved,
            mqtt5_error_reason_code_MQTT5_SHARED_SUBSCR_NOT_SUPPORTED => ErrorReasonCode::SharedSubscriptionNotSupported,
            mqtt5_error_reason_code_MQTT5_CONNECTION_RATE_EXCEEDED => ErrorReasonCode::ConnectionRateExceeded,
            mqtt5_error_reason_code_MQTT5_MAXIMUM_CONNECT_TIME => ErrorReasonCode::MaximumConnectTime,
            mqtt5_error_reason_code_MQTT5_SUBSCRIBE_IDENTIFIER_NOT_SUPPORT => ErrorReasonCode::SubscribeIdentifierNotSupported,
            mqtt5_error_reason_code_MQTT5_WILDCARD_SUBSCRIBE_NOT_SUPPORT => ErrorReasonCode::WildcardSubscriptionNotSupported,
            _ => ErrorReasonCode::UnspecifiedError,
        }
    }
}

#[cfg(esp_idf_mqtt_protocol_5)]
impl core::fmt::Display for ErrorReasonCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ErrorReasonCode::UnspecifiedError => write!(f, "Unspecified error"),
            ErrorReasonCode::MalformedPacket => write!(f, "Malformed packet"),
            ErrorReasonCode::ProtocolError => write!(f, "Protocol error"),
            ErrorReasonCode::ImplementSpecificError => write!(f, "Implementation specific error"),
            ErrorReasonCode::UnsupportedProtocolVersion => {
                write!(f, "Unsupported protocol version")
            }
            ErrorReasonCode::InvalidClientId => write!(f, "Invalid client ID"),
            ErrorReasonCode::BadUsernameOrPassword => write!(f, "Bad username or password"),
            ErrorReasonCode::NotAuthorized => write!(f, "Not authorized"),
            ErrorReasonCode::ServerUnavailable => write!(f, "Server unavailable"),
            ErrorReasonCode::ServerBusy => write!(f, "Server busy"),
            ErrorReasonCode::Banned => write!(f, "Banned"),
            ErrorReasonCode::ServerShuttingDown => write!(f, "Server shutting down"),
            ErrorReasonCode::BadAuthMethod => write!(f, "Bad authentication method"),
            ErrorReasonCode::KeepAliveTimeout => write!(f, "Keep alive timeout"),
            ErrorReasonCode::SessionTakenOver => write!(f, "Session taken over"),
            ErrorReasonCode::TopicFilterInvalid => write!(f, "Topic filter invalid"),
            ErrorReasonCode::TopicNameInvalid => write!(f, "Topic name invalid"),
            ErrorReasonCode::PacketIdentifierInUse => write!(f, "Packet identifier in use"),
            ErrorReasonCode::PacketIdentifierNotFound => write!(f, "Packet identifier not found"),
            ErrorReasonCode::ReceiveMaximumExceeded => write!(f, "Receive maximum exceeded"),
            ErrorReasonCode::TopicAliasInvalid => write!(f, "Topic alias invalid"),
            ErrorReasonCode::PacketTooLarge => write!(f, "Packet too large"),
            ErrorReasonCode::MessageRateTooHigh => write!(f, "Message rate too high"),
            ErrorReasonCode::QuotaExceeded => write!(f, "Quota exceeded"),
            ErrorReasonCode::AdministrativeAction => write!(f, "Administrative action"),
            ErrorReasonCode::PayloadFormatInvalid => write!(f, "Payload format invalid"),
            ErrorReasonCode::RetainNotSupported => write!(f, "Retain not supported"),
            ErrorReasonCode::QosNotSupported => write!(f, "QoS not supported"),
            ErrorReasonCode::UseAnotherServer => write!(f, "Use another server"),
            ErrorReasonCode::ServerMoved => write!(f, "Server moved"),
            ErrorReasonCode::SharedSubscriptionNotSupported => {
                write!(f, "Shared subscription not supported")
            }
            ErrorReasonCode::ConnectionRateExceeded => write!(f, "Connection rate exceeded"),
            ErrorReasonCode::MaximumConnectTime => write!(f, "Maximum connect time"),
            ErrorReasonCode::SubscribeIdentifierNotSupported => {
                write!(f, "Subscribe identifier not supported")
            }
            ErrorReasonCode::WildcardSubscriptionNotSupported => {
                write!(f, "Wildcard subscription not supported")
            }
        }
    }
}

impl ErrorReasonCode {
    /// Returns the numeric code value for this error reason
    pub fn code(&self) -> u32 {
        *self as u32
    }

    /// Returns true if this is a client-side error (codes 0x80-0x8F)
    pub fn is_client_error(&self) -> bool {
        (*self as u32) <= 0x8F
    }

    /// Returns true if this is a server-side error (codes 0x90+)
    pub fn is_server_error(&self) -> bool {
        (*self as u32) >= 0x90
    }

    /// Returns true if this error indicates the connection should be retried
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ErrorReasonCode::ServerUnavailable
                | ErrorReasonCode::ServerBusy
                | ErrorReasonCode::UseAnotherServer
                | ErrorReasonCode::ConnectionRateExceeded
        )
    }
}

impl EspEventProperty {
    pub(crate) fn event_property<'a, E>(
        ptr: *mut esp_mqtt5_event_property_t,
    ) -> Option<EventProperty<'a, E>> {
        if ptr.is_null() {
            None
        } else {
            let payload_format_indicator = unsafe { (*ptr).payload_format_indicator };
            let response_topic = unsafe {
                let topic = (*ptr).response_topic;
                if topic.is_null() {
                    None
                } else {
                    Some(core::ffi::CStr::from_ptr(topic).to_str().unwrap())
                }
            };

            let correlation_data = unsafe {
                let data = (*ptr).correlation_data;
                if data.is_null() {
                    None
                } else {
                    Some(core::slice::from_raw_parts(
                        data,
                        (*ptr).correlation_data_len as usize,
                    ))
                }
            };

            let content_type = unsafe {
                let content_type = (*ptr).content_type;
                if content_type.is_null() {
                    None
                } else {
                    Some(core::ffi::CStr::from_ptr(content_type).to_str().unwrap())
                }
            };

            let subscribe_id = unsafe { (*ptr).subscribe_id };

            let event_property = EventProperty::new(
                payload_format_indicator,
                response_topic,
                correlation_data,
                content_type,
                subscribe_id,
            );
            Some(event_property)
        }
    }
}

impl EspUserPropertyList {
    fn from_ptr(ptr: mqtt5_user_property_handle_t) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(EspUserPropertyList(ptr))
        }
    }

    pub(crate) fn as_ptr(&self) -> mqtt5_user_property_handle_t {
        self.0
    }
}

fn to_user_property(item: esp_mqtt5_user_property_item_t) -> UserPropertyItem {
    let key = unsafe { core::ffi::CStr::from_ptr(item.key) }
        .to_str()
        .unwrap()
        .to_string();
    let value = unsafe { core::ffi::CStr::from_ptr(item.value) }
        .to_str()
        .unwrap()
        .to_string();
    UserPropertyItem { key, value }
}

impl UserPropertyList<EspError> for EspUserPropertyList {
    fn set_items(&mut self, properties: &[UserPropertyItem]) -> Result<(), EspError> {
        let mut items: Vec<esp_mqtt5_user_property_item_t> = properties
            .iter()
            .map(|item| {
                let key_cstr = CString::new(item.key.as_str()).unwrap();
                let value_cstr = CString::new(item.value.as_str()).unwrap();

                let item = esp_mqtt5_user_property_item_t {
                    key: key_cstr.as_ptr(),
                    value: value_cstr.as_ptr(),
                };
                item
            })
            .collect();

        let error = unsafe {
            let items_ptr = items.as_mut_ptr();
            let result =
                esp_mqtt5_client_set_user_property(&mut self.0, items_ptr, items.len() as u8);
            result
        };
        esp!(error)?;
        Ok(())
    }

    fn get_items(&self) -> Result<Option<Vec<UserPropertyItem>>, EspError> {
        let count = unsafe { esp_mqtt5_client_get_user_property_count(self.0) };
        if count == 0 {
            return Ok(None);
        }
        let mut items: Vec<esp_mqtt5_user_property_item_t> = Vec::with_capacity(count as usize);
        items.resize_with(count as usize, || esp_mqtt5_user_property_item_t {
            key: core::ptr::null(),
            value: core::ptr::null(),
        });
        let error = unsafe {
            esp_mqtt5_client_get_user_property(
                self.0,
                items.as_mut_ptr(),
                &mut items.len() as *mut usize as *mut u8,
            )
        };
        esp!(error)?;
        let result: Vec<UserPropertyItem> = items.into_iter().map(to_user_property).collect();
        Ok(Some(result))
    }

    fn clear(&self) {
        unsafe {
            esp_mqtt5_client_delete_user_property(self.0);
        }
    }
}
