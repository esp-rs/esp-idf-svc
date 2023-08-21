use std::sync::{Arc, RwLock};

use ::log::{debug, info, warn};

use esp_idf_sys::*;

use crate::ble::gatt_server::service::Service;
use crate::ble::utilities::{AttributeControl, BleUuid};

/// Represents a GATT profile.
///
/// # Notes
///
/// Grouping services into a profile won't changed the actual exposed interface.
/// In this context, a profile is also called "application" in the ESP-IDF documentation.
///
/// Internally, grouping services into different profiles only defines different event handlers.
#[derive(Debug, Clone)]
pub struct Profile {
    name: Option<String>,
    pub(crate) services: Vec<Arc<RwLock<Service>>>,
    pub(crate) identifier: u16,
    pub(crate) interface: Option<u8>,
}

impl Profile {
    /// Creates a new [`Profile`].
    #[must_use]
    pub const fn new(identifier: u16) -> Self {
        Self {
            name: None,
            services: Vec::new(),
            identifier,
            interface: None,
        }
    }

    /// Sets the name of the [`Profile`].
    ///
    /// This name is only used for debugging purposes.
    pub fn name<S: Into<String>>(&mut self, name: S) -> &mut Self {
        self.name = Some(name.into());
        self
    }

    /// Adds a [`Service`] to the [`Profile`].
    #[must_use]
    pub fn service(&mut self, service: &Arc<RwLock<Service>>) -> &mut Self {
        self.services.push(service.clone());
        self
    }

    /// Returns a reference to the built [`Profile`] behind an `Arc` and an `RwLock`.
    ///
    /// The returned value can be passed to any function of this crate that expects a [`Profile`].
    /// It can be used in different threads, because it is protected by an `RwLock`.
    #[must_use]
    pub fn build(&self) -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(self.clone()))
    }

    pub(crate) fn get_service(&self, handle: u16) -> Option<Arc<RwLock<Service>>> {
        for service in &self.services {
            if service.read().unwrap().handle == Some(handle) {
                return Some(service.clone());
            }
        }

        None
    }

    pub(crate) fn get_service_by_id(&self, id: esp_gatt_id_t) -> Option<Arc<RwLock<Service>>> {
        for service in &self.services {
            if service.read().unwrap().uuid == id.into() {
                return Some(service.clone());
            }
        }

        None
    }

    pub(crate) fn register_self(&self) {
        debug!("Registering {}.", self);
        unsafe { esp_nofail!(esp_ble_gatts_app_register(self.identifier)) };
    }

    pub(crate) fn register_services(&mut self) {
        debug!("Registering {}'s services.", &self);
        let interface = self.interface.unwrap();
        self.services.iter_mut().for_each(|service| {
            service.write().unwrap().register_self(interface);
        });
    }

    pub(crate) fn on_char_add_descr(
        &mut self,
        param: esp_ble_gatts_cb_param_t_gatts_add_char_descr_evt_param,
    ) {
        // ATTENTION: Descriptors might have duplicate UUIDs!
        // We need to set them in order of creation.

        let Some(service) = self.get_service(param.service_handle) else {
            warn!("Cannot find service described by handle 0x{:04x} received in descriptor creation event.", param.service_handle);
            return;
        };

        let descriptors = service
            .read()
            .unwrap()
            .get_descriptors_by_id(param.descr_uuid);

        let Some(descriptor) = descriptors
            .iter()
            .find(|d| d.read().unwrap().attribute_handle.is_none())
        else {
            warn!("Cannot find service described by identifier {} received in descriptor creation event.", BleUuid::from(param.descr_uuid));
            return;
        };

        if param.status == esp_gatt_status_t_ESP_GATT_OK {
            info!(
                "GATT descriptor {} registered at attribute handle 0x{:04x}.",
                descriptor.read().unwrap(),
                param.attr_handle
            );
            descriptor.write().unwrap().attribute_handle = Some(param.attr_handle);
        } else {
            warn!("GATT descriptor registration failed.");
        }
    }

    pub(crate) fn on_char_add(&mut self, param: esp_ble_gatts_cb_param_t_gatts_add_char_evt_param) {
        let Some(service) = self.get_service(param.service_handle) else {
            warn!("Cannot find service described by handle 0x{:04x} received in characteristic creation event.", param.service_handle);
            return;
        };

        let Some(characteristic) = service
            .read()
            .unwrap()
            .get_characteristic_by_id(param.char_uuid)
        else {
            warn!("Cannot find characteristic described by service handle 0x{:04x} and characteristic identifier {} received in characteristic creation event.", param.service_handle, BleUuid::from(param.char_uuid));
            return;
        };

        if param.status == esp_gatt_status_t_ESP_GATT_OK {
            info!(
                "GATT characteristic {} registered at attribute handle 0x{:04x}.",
                characteristic.read().unwrap(),
                param.attr_handle
            );
            characteristic.write().unwrap().attribute_handle = Some(param.attr_handle);
            characteristic.write().unwrap().register_descriptors();
        } else {
            warn!("GATT characteristic registration failed.");
        }
    }

    pub(crate) fn on_create(&mut self, param: esp_ble_gatts_cb_param_t_gatts_create_evt_param) {
        let Some(service) = self.get_service_by_id(param.service_id.id) else {
            warn!("Cannot find service with service identifier {} received in service creation event.", BleUuid::from(param.service_id.id));
            return;
        };

        service.write().unwrap().handle = Some(param.service_handle);

        if param.status == esp_gatt_status_t_ESP_GATT_OK {
            info!(
                "GATT service {} registered on handle 0x{:04x}.",
                service.read().unwrap(),
                service.read().unwrap().handle.unwrap()
            );

            unsafe {
                esp_nofail!(esp_ble_gatts_start_service(
                    service.read().unwrap().handle.unwrap()
                ));
            }

            service.write().unwrap().register_characteristics();
        } else {
            warn!("GATT service registration failed.");
        }
    }

    pub(crate) fn on_read(
        &mut self,
        gatts_if: esp_gatt_if_t,
        param: esp_ble_gatts_cb_param_t_gatts_read_evt_param,
    ) {
        for service in &self.services {
            service
                .read()
                .unwrap()
                .characteristics
                .iter()
                .for_each(|characteristic| {
                    if characteristic.read().unwrap().attribute_handle == Some(param.handle) {
                        debug!(
                            "Received read event for characteristic {}.",
                            characteristic.read().unwrap()
                        );

                        // If the characteristic has a read handler, call it.
                        if let AttributeControl::ResponseByApp(callback) =
                            &characteristic.read().unwrap().control
                        {
                            let value = callback(param);

                            // Extend the response to the maximum length.
                            let mut response = [0u8; 600];
                            response[..value.len()].copy_from_slice(&value);

                            let mut esp_rsp = esp_gatt_rsp_t {
                                attr_value: esp_gatt_value_t {
                                    auth_req: 0,
                                    handle: param.handle,
                                    len: value.len() as u16,
                                    offset: 0,
                                    value: response,
                                },
                            };

                            unsafe {
                                esp_nofail!(esp_ble_gatts_send_response(
                                    gatts_if,
                                    param.conn_id,
                                    param.trans_id,
                                    // TODO: Allow different statuses.
                                    esp_gatt_status_t_ESP_GATT_OK,
                                    &mut esp_rsp
                                ));
                            }
                        }
                    } else {
                        characteristic
                            .read()
                            .unwrap()
                            .descriptors
                            .iter()
                            .for_each(|descriptor| {
                                debug!(
                                    "MCC: Checking descriptor {} ({:?}).",
                                    descriptor.read().unwrap(),
                                    descriptor.read().unwrap().attribute_handle
                                );

                                if descriptor.read().unwrap().attribute_handle == Some(param.handle)
                                {
                                    debug!(
                                        "Received read event for descriptor {}.",
                                        descriptor.read().unwrap()
                                    );

                                    if let AttributeControl::ResponseByApp(callback) =
                                        &descriptor.read().unwrap().control
                                    {
                                        let value = callback(param);

                                        // Extend the response to the maximum length.
                                        let mut response = [0u8; 600];
                                        response[..value.len()].copy_from_slice(&value);

                                        let mut esp_rsp = esp_gatt_rsp_t {
                                            attr_value: esp_gatt_value_t {
                                                auth_req: 0,
                                                handle: param.handle,
                                                len: value.len() as u16,
                                                offset: 0,
                                                value: response,
                                            },
                                        };

                                        unsafe {
                                            esp_nofail!(esp_ble_gatts_send_response(
                                                gatts_if,
                                                param.conn_id,
                                                param.trans_id,
                                                esp_gatt_status_t_ESP_GATT_OK,
                                                &mut esp_rsp
                                            ));
                                        }
                                    }
                                }
                            });
                    }
                });
        }
    }

    pub(crate) fn on_reg(&mut self, param: esp_ble_gatts_cb_param_t_gatts_reg_evt_param) {
        // Check status
        if param.status == esp_bt_status_t_ESP_BT_STATUS_SUCCESS {
            info!(
                "{} registered on interface {}.",
                &self,
                self.interface.unwrap()
            );
            self.register_services();
        } else {
            warn!("GATT profile registration failed.");
        }
    }

    pub(crate) fn on_start(&mut self, param: esp_ble_gatts_cb_param_t_gatts_start_evt_param) {
        let Some(service) = self.get_service(param.service_handle) else {
            warn!(
                "Cannot find service described by service handle {} received in start event.",
                param.service_handle
            );
            return;
        };

        if param.status == esp_gatt_status_t_ESP_GATT_OK {
            debug!("GATT service {} started.", service.read().unwrap());
        } else {
            warn!("GATT service {} failed to start.", service.read().unwrap());
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn on_write(
        &mut self,
        gatts_if: esp_gatt_if_t,
        param: esp_ble_gatts_cb_param_t_gatts_write_evt_param,
    ) {
        for service in &self.services {
            service
                .read()
                .unwrap()
                .characteristics
                .iter()
                .for_each(|characteristic| {
                    if characteristic.read().unwrap().attribute_handle == Some(param.handle) {
                        debug!(
                            "Received write event for characteristic {}.",
                            characteristic.read().unwrap()
                        );

                        // If the characteristic has a write handler, call it.
                        if let Some(write_callback) = &characteristic.read().unwrap().write_callback
                        {
                            let value = unsafe {
                                core::slice::from_raw_parts(param.value, param.len as usize)
                            }
                            .to_vec();

                            write_callback(value, param);

                            // Send response if needed.
                            if param.need_rsp {
                                if let AttributeControl::ResponseByApp(read_callback) =
                                    &characteristic.read().unwrap().control
                                {
                                    // Simulate a read operation.
                                    let param_as_read_operation =
                                        esp_ble_gatts_cb_param_t_gatts_read_evt_param {
                                            bda: param.bda,
                                            conn_id: param.conn_id,
                                            handle: param.handle,
                                            need_rsp: param.need_rsp,
                                            offset: param.offset,
                                            trans_id: param.trans_id,
                                            ..Default::default()
                                        };

                                    // Get value.
                                    let value = read_callback(param_as_read_operation);

                                    // Extend the response to the maximum length.
                                    let mut response = [0u8; 600];
                                    response[..value.len()].copy_from_slice(&value);

                                    let mut esp_rsp = esp_gatt_rsp_t {
                                        attr_value: esp_gatt_value_t {
                                            auth_req: 0,
                                            handle: param.handle,
                                            len: value.len() as u16,
                                            offset: 0,
                                            value: response,
                                        },
                                    };

                                    unsafe {
                                        esp_nofail!(esp_ble_gatts_send_response(
                                            gatts_if,
                                            param.conn_id,
                                            param.trans_id,
                                            esp_gatt_status_t_ESP_GATT_OK,
                                            &mut esp_rsp
                                        ));
                                    }
                                }
                            }
                        }
                    } else {
                        characteristic
                            .read()
                            .unwrap()
                            .descriptors
                            .iter()
                            .for_each(|descriptor| {
                                if descriptor.read().unwrap().attribute_handle == Some(param.handle)
                                {
                                    debug!(
                                        "Received write event for descriptor {}.",
                                        descriptor.read().unwrap()
                                    );

                                    if let Some(write_callback) =
                                        descriptor.read().unwrap().write_callback.clone()
                                    {
                                        let value = unsafe {
                                            core::slice::from_raw_parts(
                                                param.value,
                                                param.len as usize,
                                            )
                                        }
                                        .to_vec();

                                        write_callback(value, param);

                                        // Send response if needed.
                                        if param.need_rsp {
                                            if let AttributeControl::ResponseByApp(read_callback) =
                                                &descriptor.read().unwrap().control
                                            {
                                                // Simulate a read operation.
                                                let param_as_read_operation =
                                                    esp_ble_gatts_cb_param_t_gatts_read_evt_param {
                                                        bda: param.bda,
                                                        conn_id: param.conn_id,
                                                        handle: param.handle,
                                                        need_rsp: param.need_rsp,
                                                        offset: param.offset,
                                                        trans_id: param.trans_id,
                                                        ..Default::default()
                                                    };

                                                // Get value.
                                                let value = read_callback(param_as_read_operation);

                                                // Extend the response to the maximum length.
                                                let mut response = [0u8; 600];
                                                response[..value.len()].copy_from_slice(&value);

                                                let mut esp_rsp = esp_gatt_rsp_t {
                                                    attr_value: esp_gatt_value_t {
                                                        auth_req: 0,
                                                        handle: param.handle,
                                                        len: value.len() as u16,
                                                        offset: 0,
                                                        value: response,
                                                    },
                                                };

                                                unsafe {
                                                    esp_nofail!(esp_ble_gatts_send_response(
                                                        gatts_if,
                                                        param.conn_id,
                                                        param.trans_id,
                                                        esp_gatt_status_t_ESP_GATT_OK,
                                                        &mut esp_rsp
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            });
                    }
                });
        }
    }

    /// Profile-specific GATT server event loop.
    pub(crate) fn gatts_event_handler(
        &mut self,
        event: esp_gatts_cb_event_t,
        gatts_if: esp_gatt_if_t,
        param: *mut esp_ble_gatts_cb_param_t,
    ) {
        #[allow(non_upper_case_globals)]
        match event {
            esp_gatts_cb_event_t_ESP_GATTS_REG_EVT => {
                let param = unsafe { (*param).reg };

                self.on_reg(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_CREATE_EVT => {
                let param = unsafe { (*param).create };

                self.on_create(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_START_EVT => {
                let param = unsafe { (*param).start };

                self.on_start(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_ADD_CHAR_EVT => {
                let param = unsafe { (*param).add_char };

                self.on_char_add(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_ADD_CHAR_DESCR_EVT => {
                let param = unsafe { (*param).add_char_descr };

                self.on_char_add_descr(param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_WRITE_EVT => {
                let param = unsafe { (*param).write };

                self.on_write(gatts_if, param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_READ_EVT => {
                let param = unsafe { (*param).read };

                self.on_read(gatts_if, param);
            }
            esp_gatts_cb_event_t_ESP_GATTS_CONF_EVT => {
                let _param = unsafe { (*param).conf };

                // TODO: on_conf.
                debug!("Received confirmation event.");
            }
            _ => {
                warn!("Unhandled GATT server event: {:?}", event);
            }
        }
    }
}

impl core::fmt::Display for Profile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} (0x{:04x})",
            self.name.as_deref().unwrap_or_else(|| "Unnamed profile"),
            self.identifier,
        )
    }
}
