//! MQTT protocol
//!
//! MQTT is a lightweight publish/subscribe messaging protocol.

pub mod client;

#[cfg(all(esp_idf_mqtt_protocol_5, feature = "std"))]
pub mod client5;
