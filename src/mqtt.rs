//! MQTT protocol
//!
//! MQTT is a lightweight publish/subscribe messaging protocol.

pub mod client;

#[cfg(all(feature = "std", esp_idf_mqtt_protocol_5))]
pub mod client5;
