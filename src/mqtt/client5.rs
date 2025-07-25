#[cfg(esp_idf_mqtt_protocol_5)]


use crate::sys::*;


#[allow(unused_imports)]
pub use super::*;

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
impl TryFrom<mqtt5_error_reason_code> for ErrorReasonCode {
    type Error = ();

    fn try_from(code: mqtt5_error_reason_code) -> Result<Self, Self::Error> {

        match code {
            mqtt5_error_reason_code_MQTT5_UNSPECIFIED_ERROR => Ok(ErrorReasonCode::UnspecifiedError),
            mqtt5_error_reason_code_MQTT5_MALFORMED_PACKET => Ok(ErrorReasonCode::MalformedPacket),
            mqtt5_error_reason_code_MQTT5_PROTOCOL_ERROR => Ok(ErrorReasonCode::ProtocolError),
            mqtt5_error_reason_code_MQTT5_IMPLEMENT_SPECIFIC_ERROR => Ok(ErrorReasonCode::ImplementSpecificError),
            mqtt5_error_reason_code_MQTT5_UNSUPPORTED_PROTOCOL_VER => Ok(ErrorReasonCode::UnsupportedProtocolVersion),
            mqtt5_error_reason_code_MQTT5_INVALID_CLIENT_ID => Ok(ErrorReasonCode::InvalidClientId),
            mqtt5_error_reason_code_MQTT5_BAD_USERNAME_OR_PWD => Ok(ErrorReasonCode::BadUsernameOrPassword),
            mqtt5_error_reason_code_MQTT5_NOT_AUTHORIZED => Ok(ErrorReasonCode::NotAuthorized),
            mqtt5_error_reason_code_MQTT5_SERVER_UNAVAILABLE => Ok(ErrorReasonCode::ServerUnavailable),
            mqtt5_error_reason_code_MQTT5_SERVER_BUSY => Ok(ErrorReasonCode::ServerBusy),
            mqtt5_error_reason_code_MQTT5_BANNED => Ok(ErrorReasonCode::Banned),
            mqtt5_error_reason_code_MQTT5_SERVER_SHUTTING_DOWN => Ok(ErrorReasonCode::ServerShuttingDown),
            mqtt5_error_reason_code_MQTT5_BAD_AUTH_METHOD => Ok(ErrorReasonCode::BadAuthMethod),
            mqtt5_error_reason_code_MQTT5_KEEP_ALIVE_TIMEOUT => Ok(ErrorReasonCode::KeepAliveTimeout),
            mqtt5_error_reason_code_MQTT5_SESSION_TAKEN_OVER => Ok(ErrorReasonCode::SessionTakenOver),
            mqtt5_error_reason_code_MQTT5_TOPIC_FILTER_INVALID => Ok(ErrorReasonCode::TopicFilterInvalid),
            mqtt5_error_reason_code_MQTT5_TOPIC_NAME_INVALID => Ok(ErrorReasonCode::TopicNameInvalid),
            mqtt5_error_reason_code_MQTT5_PACKET_IDENTIFIER_IN_USE => Ok(ErrorReasonCode::PacketIdentifierInUse),
            mqtt5_error_reason_code_MQTT5_PACKET_IDENTIFIER_NOT_FOUND => Ok(ErrorReasonCode::PacketIdentifierNotFound),
            mqtt5_error_reason_code_MQTT5_RECEIVE_MAXIMUM_EXCEEDED => Ok(ErrorReasonCode::ReceiveMaximumExceeded),
            mqtt5_error_reason_code_MQTT5_TOPIC_ALIAS_INVALID => Ok(ErrorReasonCode::TopicAliasInvalid),
            mqtt5_error_reason_code_MQTT5_PACKET_TOO_LARGE => Ok(ErrorReasonCode::PacketTooLarge),
            mqtt5_error_reason_code_MQTT5_MESSAGE_RATE_TOO_HIGH => Ok(ErrorReasonCode::MessageRateTooHigh),
            mqtt5_error_reason_code_MQTT5_QUOTA_EXCEEDED => Ok(ErrorReasonCode::QuotaExceeded),
            mqtt5_error_reason_code_MQTT5_ADMINISTRATIVE_ACTION => Ok(ErrorReasonCode::AdministrativeAction),
            mqtt5_error_reason_code_MQTT5_PAYLOAD_FORMAT_INVALID => Ok(ErrorReasonCode::PayloadFormatInvalid),
            mqtt5_error_reason_code_MQTT5_RETAIN_NOT_SUPPORT => Ok(ErrorReasonCode::RetainNotSupported),
            mqtt5_error_reason_code_MQTT5_QOS_NOT_SUPPORT => Ok(ErrorReasonCode::QosNotSupported),
            mqtt5_error_reason_code_MQTT5_USE_ANOTHER_SERVER => Ok(ErrorReasonCode::UseAnotherServer),
            mqtt5_error_reason_code_MQTT5_SERVER_MOVED => Ok(ErrorReasonCode::ServerMoved),
            mqtt5_error_reason_code_MQTT5_SHARED_SUBSCR_NOT_SUPPORTED => Ok(ErrorReasonCode::SharedSubscriptionNotSupported),
            mqtt5_error_reason_code_MQTT5_CONNECTION_RATE_EXCEEDED => Ok(ErrorReasonCode::ConnectionRateExceeded),
            mqtt5_error_reason_code_MQTT5_MAXIMUM_CONNECT_TIME => Ok(ErrorReasonCode::MaximumConnectTime),
            mqtt5_error_reason_code_MQTT5_SUBSCRIBE_IDENTIFIER_NOT_SUPPORT => Ok(ErrorReasonCode::SubscribeIdentifierNotSupported),
            mqtt5_error_reason_code_MQTT5_WILDCARD_SUBSCRIBE_NOT_SUPPORT => Ok(ErrorReasonCode::WildcardSubscriptionNotSupported),
            _ => Err(()),
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
            ErrorReasonCode::UnsupportedProtocolVersion => write!(f, "Unsupported protocol version"),
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
            ErrorReasonCode::SharedSubscriptionNotSupported => write!(f, "Shared subscription not supported"),
            ErrorReasonCode::ConnectionRateExceeded => write!(f, "Connection rate exceeded"),
            ErrorReasonCode::MaximumConnectTime => write!(f, "Maximum connect time"),
            ErrorReasonCode::SubscribeIdentifierNotSupported => write!(f, "Subscribe identifier not supported"),
            ErrorReasonCode::WildcardSubscriptionNotSupported => write!(f, "Wildcard subscription not supported"),
        }
    }
}

#[cfg(esp_idf_mqtt_protocol_5)]
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