//! MQTT protocol
//!
//! MQTT is a lightweight publish/subscribe messaging protocol.

pub mod client;

#[cfg(esp_idf_mqtt_protocol_5)]
pub mod client5;
