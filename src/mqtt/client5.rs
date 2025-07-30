use core::ffi::c_void;
use std::{boxed::Box, vec::Vec};

use crate::{
    handle::RawHandle,
    mqtt::client::{EspMqttConnection, EspMqttEvent, MqttClientConfiguration, UnsafeCallback},
    private::{
        cstr::{to_cstring_arg, RawCstrs},
        zerocopy::{Channel, QuitOnDrop},
    },
    tls::TlsPsk,
};

#[allow(unused_imports)]
pub use super::*;

extern crate alloc;
use alloc::ffi::CString;

use embedded_svc::mqtt::{
    client::{Enqueue, ErrorType},
    client5::{
        Client, DisconnectPropertyConfig, Publish, UnsubscribePropertyConfig, UserPropertyItem,
        UserPropertyList,
    },
};
#[cfg(all(esp_idf_mqtt_protocol_5, feature = "std"))]
use embedded_svc::mqtt::{
    client::{MessageId, QoS},
    client5::{PublishPropertyConfig, SubscribePropertyConfig},
};

#[allow(unused_imports)]
use esp_idf_hal::sys::*;

pub struct EspUserPropertyList(pub(crate) mqtt5_user_property_handle_t, RawCstrs);

impl EspUserPropertyList {
    pub fn new() -> Self {
        let handle = mqtt5_user_property_handle_t::default();
        Self(handle, RawCstrs::new())
    }
    pub(crate) fn from_handle(handle: mqtt5_user_property_handle_t) -> Self {
        let raw_cstrs = RawCstrs::new();
        EspUserPropertyList(handle, raw_cstrs)
    }
    pub fn from<'a>(items: &&[UserPropertyItem<'a>]) -> Self {
        let handle = mqtt5_user_property_handle_t::default();
        let mut list = EspUserPropertyList(handle, RawCstrs::new());
        list.set_items(items)
            .expect("Failed to set user properties");
        list
    }

    pub fn as_ptr(&self) -> mqtt5_user_property_handle_t {
        self.0
    }

    pub fn as_const_ptr(&self) -> *const mqtt5_user_property_handle_t {
        self.0 as *const mqtt5_user_property_handle_t
    }

    fn count(&self) -> u8 {
        let count = unsafe { esp_mqtt5_client_get_user_property_count(self.0) };
        count
    }

    fn set_items(&mut self, properties: &[UserPropertyItem]) -> Result<(), EspError> {
        let items_result: Result<Vec<esp_mqtt5_user_property_item_t>, EspError> = properties
            .into_iter()
            .map(|item| {
                let key = self.1.as_ptr(item.key)?;
                let value = self.1.as_ptr(item.value)?;
                let item = esp_mqtt5_user_property_item_t {
                    key: key,
                    value: value,
                };
                Ok(item)
            })
            .collect();

        let mut items = items_result?;
        let error = unsafe {
            let items_ptr = items.as_mut_ptr();
            let result =
                esp_mqtt5_client_set_user_property(&mut self.0, items_ptr, items.len() as u8);
            result
        };
        esp!(error)?;
        Ok(())
    }

    fn get_items<'a>(&self) -> Result<Vec<UserPropertyItem<'a>>, EspError> {
        let count = unsafe { esp_mqtt5_client_get_user_property_count(self.0) };
        if count == 0 {
            return Ok(Vec::new());
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
        let result: Vec<UserPropertyItem<'a>> = items
            .into_iter()
            .map(|i| EspUserPropertyItem(i).into())
            .collect();
        Ok(result)
    }

    fn clear(&self) {
        unsafe {
            esp_mqtt5_client_delete_user_property(self.0);
        }
    }
}

pub struct EspUserPropertyItem(pub(crate) esp_mqtt5_user_property_item_t);

impl<'a> From<UserPropertyItem<'a>> for EspUserPropertyItem {
    fn from(item: UserPropertyItem<'a>) -> Self {
        let key_cstr = CString::new(item.key).unwrap();
        let value_cstr = CString::new(item.value).unwrap();

        EspUserPropertyItem(esp_mqtt5_user_property_item_t {
            key: key_cstr.as_ptr(),
            value: value_cstr.as_ptr(),
        })
    }
}

impl<'a> Into<UserPropertyItem<'a>> for EspUserPropertyItem {
    fn into(self) -> UserPropertyItem<'a> {
        let key: &'a str = unsafe {
            if self.0.key.is_null() {
                ""
            } else {
                std::ffi::CStr::from_ptr(self.0.key).to_str().unwrap_or("")
            }
        };
        let value: &'a str = unsafe {
            if self.0.value.is_null() {
                ""
            } else {
                std::ffi::CStr::from_ptr(self.0.value)
                    .to_str()
                    .unwrap_or("")
            }
        };
        UserPropertyItem { key, value }
    }
}

impl UserPropertyList<EspError> for EspUserPropertyList {
    fn set_items(&mut self, properties: &[UserPropertyItem]) -> Result<(), EspError> {
        EspUserPropertyList::set_items(self, properties)
    }

    fn get_items<'a>(&self) -> Result<Vec<UserPropertyItem<'a>>, EspError> {
        EspUserPropertyList::get_items(self)
    }

    fn clear(&self) {
        EspUserPropertyList::clear(self)
    }

    fn count(&self) -> u8 {
        EspUserPropertyList::count(self)
    }
}

impl UserPropertyList<EspError> for &EspUserPropertyList {
    fn set_items(&mut self, properties: &[UserPropertyItem]) -> Result<(), EspError> {
        // SAFETY: The caller must guarantee exclusive access when calling set_items via &self.
        let mut_self = unsafe { &mut *(self as *const _ as *mut EspUserPropertyList) };
        EspUserPropertyList::set_items(mut_self, properties)
    }

    fn get_items<'a>(&self) -> Result<Vec<UserPropertyItem<'a>>, EspError> {
        EspUserPropertyList::get_items(self)
    }

    fn clear(&self) {
        EspUserPropertyList::clear(self)
    }

    fn count(&self) -> u8 {
        EspUserPropertyList::count(self)
    }
}

struct EspPublishPropertyConfig(
    pub(crate) esp_mqtt5_publish_property_config_t,
    Option<EspUserPropertyList>,
    #[allow(dead_code)] Option<RawCstrs>, //Holds the C strings to prevent them from being dropped prematurely
);

impl TryFrom<Option<PublishPropertyConfig<'_>>> for EspPublishPropertyConfig {
    type Error = EspError;
    fn try_from(config: Option<PublishPropertyConfig<'_>>) -> Result<Self, EspError> {
        let mut cstrs = RawCstrs::new();
        if config.is_none() {
            // If no config is provided, we use an empty config
            return Ok(EspPublishPropertyConfig(
                esp_mqtt5_publish_property_config_t::default(),
                None,
                None,
            ));
        }
        let config = config.unwrap();
        let mut user_properties = EspUserPropertyList::new();

        if let Some(ref user_properties_items) = config.user_properties {
            user_properties.set_items(user_properties_items)?;
        }

        let property = esp_mqtt5_publish_property_config_t {
            payload_format_indicator: config.payload_format_indicator,
            topic_alias: config.topic_alias,
            message_expiry_interval: config.message_expiry_interval,
            response_topic: cstrs.as_nptr(config.response_topic)?,
            correlation_data: config
                .correlation_data
                .map_or(core::ptr::null(), |s| s.as_ptr()),
            correlation_data_len: config.correlation_data.map_or(0, |s| s.len() as _),
            content_type: cstrs.as_nptr(config.content_type)?,
            user_property: user_properties.as_ptr(),
        };
        //transfer ownership of cstrs to the struct
        Ok(EspPublishPropertyConfig(
            property,
            Some(user_properties),
            Some(cstrs),
        ))
    }
}

impl Drop for EspPublishPropertyConfig {
    fn drop(&mut self) {
        // Clear the user properties if they were set
        if let Some(ref user_properties) = self.1 {
            user_properties.clear();
        }
    }
}

struct EspSubscribePropertyConfig(
    pub(crate) esp_mqtt5_subscribe_property_config_t,
    Option<EspUserPropertyList>,
    #[allow(dead_code)] Option<RawCstrs>,
);

impl<'a> TryFrom<Option<SubscribePropertyConfig<'a>>> for EspSubscribePropertyConfig {
    type Error = EspError;
    fn try_from(config: Option<SubscribePropertyConfig<'a>>) -> Result<Self, EspError> {
        if config.is_none() {
            // If no config is provided, we use an empty config
            return Ok(EspSubscribePropertyConfig(
                esp_mqtt5_subscribe_property_config_t::default(),
                None,
                None,
            ));
        }

        let config = config.unwrap();
        let mut cstrs = RawCstrs::new();
        let mut user_properties = EspUserPropertyList::new();
        if let Some(ref user_properties_items) = config.user_properties {
            user_properties.set_items(user_properties_items)?;
        }

        let property = esp_mqtt5_subscribe_property_config_t {
            no_local_flag: config.no_local,
            retain_as_published_flag: config.retain_as_published,
            retain_handle: config.retain_handling as _,
            is_share_subscribe: config.share_name.is_some(),
            share_name: cstrs.as_nptr(config.share_name)?,
            subscribe_id: config.subscribe_id,
            user_property: user_properties.as_ptr(),
        };

        Ok(EspSubscribePropertyConfig(
            property,
            Some(user_properties),
            Some(cstrs),
        ))
    }
}

impl Drop for EspSubscribePropertyConfig {
    fn drop(&mut self) {
        // Clear the user properties if they were set
        if let Some(ref user_properties) = self.1 {
            user_properties.clear();
        }
    }
}

struct EspUnsubscribePropertyConfig(
    pub(crate) esp_mqtt5_unsubscribe_property_config_t,
    Option<EspUserPropertyList>,
    #[allow(dead_code)] Option<RawCstrs>,
);

impl<'a> TryFrom<Option<UnsubscribePropertyConfig<'a>>> for EspUnsubscribePropertyConfig {
    type Error = EspError;
    fn try_from(config: Option<UnsubscribePropertyConfig<'a>>) -> Result<Self, EspError> {
        if config.is_none() {
            // If no config is provided, we use an empty config
            return Ok(EspUnsubscribePropertyConfig(
                esp_mqtt5_unsubscribe_property_config_t::default(),
                None,
                None,
            ));
        }

        let config = config.unwrap();
        let mut cstrs = RawCstrs::new();
        let mut user_properties = EspUserPropertyList::new();
        if let Some(ref user_properties_items) = config.user_properties {
            user_properties.set_items(user_properties_items)?;
        }

        let property = esp_mqtt5_unsubscribe_property_config_t {
            is_share_subscribe: config.share_name.is_some(),
            share_name: config.share_name.map_or(core::ptr::null(), |s| s.as_ptr()),
            user_property: user_properties.as_ptr(),
        };
        Ok(EspUnsubscribePropertyConfig(
            property,
            Some(user_properties),
            Some(cstrs),
        ))
    }
}

impl Drop for EspUnsubscribePropertyConfig {
    fn drop(&mut self) {
        // Clear the user properties if they were set
        if let Some(ref user_properties) = self.1 {
            user_properties.clear();
        }
    }
}

pub struct EspDisconnectPropertyConfig(
    pub(crate) esp_mqtt5_disconnect_property_config_t,
    Option<EspUserPropertyList>,
);

impl<'a> TryFrom<Option<DisconnectPropertyConfig<'a>>> for EspDisconnectPropertyConfig {
    type Error = EspError;
    fn try_from(config: Option<DisconnectPropertyConfig<'a>>) -> Result<Self, EspError> {
        if config.is_none() {
            // If no config is provided, we use an empty config
            return Ok(EspDisconnectPropertyConfig(
                esp_mqtt5_disconnect_property_config_t::default(),
                None,
            ));
        }

        let config = config.unwrap();
        let mut user_properties = EspUserPropertyList::new();
        if let Some(ref user_properties_items) = config.user_properties {
            user_properties.set_items(user_properties_items)?;
        }
        let property = esp_mqtt5_disconnect_property_config_t {
            session_expiry_interval: config.session_expiry_interval,
            disconnect_reason: config.reason,
            user_property: user_properties.as_ptr(),
        };
        Ok(EspDisconnectPropertyConfig(property, Some(user_properties)))
    }
}

impl Drop for EspDisconnectPropertyConfig {
    fn drop(&mut self) {
        // Clear the user properties if they were set
        if let Some(ref user_properties) = self.1 {
            user_properties.clear();
        }
    }
}

pub struct EspMqtt5Client<'a> {
    raw_client: esp_mqtt_client_handle_t,
    _boxed_raw_callback: Box<dyn FnMut(esp_mqtt_event_handle_t) + Send + 'a>,
    _tls_psk_conf: Option<TlsPsk>,
}

impl RawHandle for EspMqtt5Client<'_> {
    type Handle = esp_mqtt_client_handle_t;

    fn handle(&self) -> Self::Handle {
        self.raw_client
    }
}

impl EspMqtt5Client<'static> {
    pub fn new(
        url: &str,
        conf: &MqttClientConfiguration,
    ) -> Result<(Self, EspMqttConnection), EspError>
    where
        Self: Sized,
    {
        let (channel, receiver) = Channel::new();

        let sender = QuitOnDrop::new(channel);

        let conn = EspMqttConnection {
            receiver,
            given: false,
        };

        let client = Self::new_cb(url, conf, move |mut event| {
            let event: &mut EspMqttEvent<'static> = unsafe { core::mem::transmute(&mut event) };
            sender.channel().share(event);
        })?;

        Ok((client, conn))
    }

    pub fn new_cb<F>(
        url: &str,
        conf: &MqttClientConfiguration,
        callback: F,
    ) -> Result<Self, EspError>
    where
        F: for<'b> FnMut(EspMqttEvent<'b>) + Send + 'static,
        Self: Sized,
    {
        unsafe { Self::new_nonstatic_cb(url, conf, callback) }
    }

    pub fn as_ptr(&self) -> esp_mqtt_client_handle_t {
        self.raw_client
    }
}

impl<'a> EspMqtt5Client<'a> {
    pub unsafe fn new_nonstatic_cb<F>(
        url: &str,
        conf: &MqttClientConfiguration,
        mut callback: F,
    ) -> Result<Self, EspError>
    where
        F: for<'b> FnMut(EspMqttEvent<'b>) + Send + 'a,
        Self: Sized,
    {
        Self::new_raw(
            url,
            conf,
            Box::new(move |event_handle| {
                callback(EspMqttEvent::new(unsafe { event_handle.as_ref() }.unwrap()));
            }),
        )
    }

    fn new_raw(
        url: &str,
        conf: &MqttClientConfiguration,
        raw_callback: Box<dyn FnMut(esp_mqtt_event_handle_t) + Send + 'a>,
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

        Ok(client)
    }

    pub fn subscribe<'ab>(
        &mut self,
        topic: &str,
        qos: QoS,
        config: Option<SubscribePropertyConfig<'ab>>,
    ) -> Result<MessageId, EspError> {
        self.subscribe_cstr(to_cstring_arg(topic)?.as_c_str(), qos, config)
    }

    pub fn unsubscribe<'ab>(
        &mut self,
        topic: &str,
        config: Option<UnsubscribePropertyConfig<'ab>>,
    ) -> Result<MessageId, EspError> {
        self.unsubscribe_cstr(to_cstring_arg(topic)?.as_c_str(), config)
    }

    pub fn publish<'ab>(
        &mut self,
        topic: &str,
        qos: QoS,
        retain: bool,
        payload: &[u8],
        config: Option<PublishPropertyConfig<'ab>>,
    ) -> Result<MessageId, EspError> {
        self.publish_cstr(
            to_cstring_arg(topic)?.as_c_str(),
            qos,
            retain,
            payload,
            config,
        )
    }

    pub fn enqueue(
        &mut self,
        topic: &str,
        qos: QoS,
        retain: bool,
        payload: &[u8],
    ) -> Result<MessageId, EspError> {
        self.enqueue_cstr(to_cstring_arg(topic)?.as_c_str(), qos, retain, payload)
    }

    pub fn disconnect<'ab>(
        &mut self,
        config: Option<DisconnectPropertyConfig<'ab>>,
    ) -> Result<(), EspError> {
        let property = EspDisconnectPropertyConfig::try_from(config)?;
        if config.is_some() {
            Self::check(unsafe {
                esp_mqtt5_client_set_disconnect_property(self.raw_client, &property.0 as *const _)
            })?;
        }

        Self::check(unsafe { esp_mqtt_client_disconnect(self.raw_client) })?;
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), EspError> {
        Self::check(unsafe { esp_mqtt_client_start(self.raw_client) })?;
        Ok(())
    }

    pub fn subscribe_cstr<'ab>(
        &mut self,
        topic: &core::ffi::CStr,
        qos: QoS,
        config: Option<SubscribePropertyConfig<'ab>>,
    ) -> Result<MessageId, EspError> {
        let property = EspSubscribePropertyConfig::try_from(config)?;
        if config.is_some() {
            // If no config is provided, we use an empty config
            Self::check(unsafe {
                esp_mqtt5_client_set_subscribe_property(self.raw_client, &property.0 as *const _)
            })?;
        }

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
            esp_mqtt_client_subscribe(self.raw_client, topic.as_ptr(), qos as _)
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
            esp_mqtt_client_subscribe_single(self.raw_client, topic.as_ptr(), qos as _)
        });

        res
    }

    pub fn unsubscribe_cstr<'ab>(
        &mut self,
        topic: &core::ffi::CStr,
        config: Option<UnsubscribePropertyConfig<'ab>>,
    ) -> Result<MessageId, EspError> {
        let property = EspUnsubscribePropertyConfig::try_from(config)?;
        if config.is_some() {
            // If no config is provided, we use an empty config
            Self::check(unsafe {
                esp_mqtt5_client_set_unsubscribe_property(self.raw_client, &property.0 as *const _)
            })?;
        }
        Self::check(unsafe { esp_mqtt_client_unsubscribe(self.raw_client, topic.as_ptr()) })
    }

    pub fn publish_cstr<'ab>(
        &mut self,
        topic: &core::ffi::CStr,
        qos: QoS,
        retain: bool,
        payload: &[u8],
        config: Option<PublishPropertyConfig<'ab>>,
    ) -> Result<MessageId, EspError> {
        ::log::info!(
            "Publishing to topic: {}, qos: {:?}, retain: {}, payload length: {}, config: {:?}",
            topic.to_str().unwrap_or("Invalid UTF-8"),
            qos,
            retain,
            payload.len(),
            config
        );

        let property = EspPublishPropertyConfig::try_from(config)?;

        if config.is_some() {
            Self::check(unsafe {
                esp_mqtt5_client_set_publish_property(self.raw_client, &property.0 as *const _)
            })?;
        }

        let payload_ptr = match payload.len() {
            0 => core::ptr::null(),
            _ => payload.as_ptr(),
        };

        let result = Self::check(unsafe {
            esp_mqtt_client_publish(
                self.raw_client,
                topic.as_ptr(),
                payload_ptr as _,
                payload.len() as _,
                qos as _,
                retain as _,
            )
        });
        drop(property); // Ensure the property is dropped after use
        return result;
    }

    pub fn enqueue_cstr(
        &mut self,
        topic: &core::ffi::CStr,
        qos: QoS,
        retain: bool,
        payload: &[u8],
    ) -> Result<MessageId, EspError> {
        let payload_ptr = match payload.len() {
            0 => core::ptr::null(),
            _ => payload.as_ptr(),
        };

        Self::check(unsafe {
            esp_mqtt_client_enqueue(
                self.raw_client,
                topic.as_ptr(),
                payload_ptr as _,
                payload.len() as _,
                qos as _,
                retain as _,
                true,
            )
        })
    }

    pub fn set_uri(&mut self, uri: &str) -> Result<MessageId, EspError> {
        self.set_uri_cstr(to_cstring_arg(uri)?.as_c_str())
    }

    pub fn set_uri_cstr(&mut self, uri: &core::ffi::CStr) -> Result<MessageId, EspError> {
        Self::check(unsafe { esp_mqtt_client_set_uri(self.raw_client, uri.as_ptr()) })
    }

    pub fn get_outbox_size(&self) -> usize {
        // this is always positive as internally this is converting uint64_t to int (defaults to 0)
        let outbox_size = unsafe { esp_mqtt_client_get_outbox_size(self.raw_client) };
        outbox_size.max(0) as usize
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

    fn check(result: i32) -> Result<MessageId, EspError> {
        match EspError::from(result) {
            Some(err) if result < 0 => Err(err),
            _ => Ok(result as _),
        }
    }
}

impl Drop for EspMqtt5Client<'_> {
    fn drop(&mut self) {
        unsafe {
            esp_mqtt_client_destroy(self.raw_client as _);
        }
    }
}

impl ErrorType for EspMqtt5Client<'_> {
    type Error = EspError;
}

impl<'a> Client for EspMqtt5Client<'a> {
    fn subscribe<'ab>(
        &mut self,
        topic: &str,
        qos: QoS,
        config: Option<SubscribePropertyConfig<'ab>>,
    ) -> Result<MessageId, Self::Error> {
        EspMqtt5Client::subscribe(self, topic, qos, config)
    }

    fn unsubscribe<'ab>(
        &mut self,
        topic: &str,
        config: Option<UnsubscribePropertyConfig<'ab>>,
    ) -> Result<MessageId, Self::Error> {
        EspMqtt5Client::unsubscribe(self, topic, config)
    }

    fn disconnect<'ab>(
        &mut self,
        config: Option<DisconnectPropertyConfig<'ab>>,
    ) -> Result<(), Self::Error> {
        EspMqtt5Client::disconnect(self, config)
    }
}

impl Publish for EspMqtt5Client<'_> {
    fn publish(
        &mut self,
        topic: &str,
        qos: QoS,
        retain: bool,
        payload: &[u8],
        config: Option<PublishPropertyConfig<'_>>,
    ) -> Result<MessageId, Self::Error> {
        EspMqtt5Client::publish(self, topic, qos, retain, payload, config)
    }
}

impl Enqueue for EspMqtt5Client<'_> {
    fn enqueue(
        &mut self,
        topic: &str,
        qos: QoS,
        retain: bool,
        payload: &[u8],
    ) -> Result<MessageId, Self::Error> {
        EspMqtt5Client::enqueue(self, topic, qos, retain, payload)
    }
}

unsafe impl Send for EspMqtt5Client<'_> {}
