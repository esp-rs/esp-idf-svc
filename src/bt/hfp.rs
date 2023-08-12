#[cfg(esp_idf_bt_hfp_client_enable)]
pub mod client {
    use core::{
        borrow::Borrow,
        ffi,
        marker::PhantomData,
        sync::atomic::{AtomicBool, Ordering},
    };

    use esp_idf_sys::*;
    use log::{debug, info};

    use crate::{
        bt::{BtCallback, BtClassicEnabled, BtDriver},
        private::cstr::to_cstring_arg,
    };

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub enum Volume {
        Speaker(u8),
        Microphone(u8),
    }

    #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub struct Source {
        pub sample_rate_hz: u32,
        pub bits_per_sample: u8,
        pub stereo: bool,
    }

    #[derive(Debug)]
    pub enum HfpcEvent<'a> {
        #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
        RecvData(&'a [u8]),
        #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
        SendData(&'a mut [u8]),
        Other(PhantomData<&'a ()>),
    }

    #[allow(non_upper_case_globals)]
    impl<'a> From<(esp_hf_client_cb_event_t, &'a esp_hf_client_cb_param_t)> for HfpcEvent<'a> {
        fn from(value: (esp_hf_client_cb_event_t, &'a esp_hf_client_cb_param_t)) -> Self {
            let (evt, param) = value;

            unsafe {
                match evt {
                    _ => {
                        log::warn!("Unknown event {:?}", evt);
                        Self::Other(PhantomData)
                        //panic!("Unknown event {:?}", evt)
                    }
                }
            }
        }
    }

    pub struct EspHfpc<'d, M, T>
    where
        T: Borrow<BtDriver<'d, M>>,
        M: BtClassicEnabled,
    {
        _driver: T,
        initialized: AtomicBool,
        #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
        resampling_source: Option<Source>,
        _p: PhantomData<&'d ()>,
        _m: PhantomData<M>,
    }

    impl<'d, M, T> EspHfpc<'d, M, T>
    where
        T: Borrow<BtDriver<'d, M>>,
        M: BtClassicEnabled,
    {
        #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
        pub const fn new(driver: T, resampling_source: Option<Source>) -> Result<Self, EspError> {
            Ok(Self {
                _driver: driver,
                initialized: AtomicBool::new(false),
                resampling_source,
                _p: PhantomData,
                _m: PhantomData,
            })
        }

        #[cfg(not(esp_idf_bt_hfp_audio_data_path_hci))]
        pub const fn new(driver: T) -> Result<Self, EspError> {
            Ok(Self {
                _driver: driver,
                initialized: AtomicBool::new(false),
                _p: PhantomData,
                _m: PhantomData,
            })
        }

        pub fn initialize<F>(&self, events_cb: F) -> Result<(), EspError>
        where
            F: Fn(HfpcEvent) -> usize + Send + 'd,
        {
            CALLBACK.set(events_cb)?;

            esp!(unsafe { esp_hf_client_init() })?;
            esp!(unsafe { esp_hf_client_register_callback(Some(Self::event_handler)) })?;

            #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
            esp!(unsafe {
                esp_hf_client_register_data_callback(
                    Some(Self::recv_data_handler),
                    Some(Self::send_data_handler),
                )
            })?;

            #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
            if let Some(resampling_source) = self.resampling_source {
                unsafe {
                    esp_hf_client_pcm_resample_init(
                        resampling_source.sample_rate_hz,
                        resampling_source.bits_per_sample as _,
                        if resampling_source.stereo { 2 } else { 1 },
                    );
                }
            }

            self.initialized.store(true, Ordering::SeqCst);

            Ok(())
        }

        pub fn connect(&self, remote_bda: &esp_bd_addr_t) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_connect(remote_bda as *const _ as *mut _) })
        }

        pub fn disconnect(&self, remote_bda: &esp_bd_addr_t) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_disconnect(remote_bda as *const _ as *mut _) })
        }

        pub fn connect_audio(&self, remote_bda: &esp_bd_addr_t) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_connect_audio(remote_bda as *const _ as *mut _) })
        }

        pub fn disconnect_audio(&self, remote_bda: &esp_bd_addr_t) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_disconnect_audio(remote_bda as *const _ as *mut _) })
        }

        pub fn start_voice_recognition(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_start_voice_recognition() })
        }

        pub fn stop_voice_recognition(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_stop_voice_recognition() })
        }

        pub fn update_volume(&self, volume: Volume) -> Result<(), EspError> {
            let (volume_type, gain) = match volume {
                Volume::Speaker(gain) => (
                    esp_hf_volume_control_target_t_ESP_HF_VOLUME_CONTROL_TARGET_SPK,
                    gain as _,
                ),
                Volume::Microphone(gain) => (
                    esp_hf_volume_control_target_t_ESP_HF_VOLUME_CONTROL_TARGET_MIC,
                    gain as _,
                ),
            };

            esp!(unsafe { esp_hf_client_volume_update(volume_type, gain) })
        }

        pub fn dial(&self, number: &str) -> Result<(), EspError> {
            let number = to_cstring_arg(number)?;

            esp!(unsafe { esp_hf_client_dial(number.as_ptr()) })
        }

        pub fn dial_memory(&self, location: usize) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_dial_memory(location as _) })
        }

        // pub fn hold(&self, location: usize) -> Result<(), EspError> {
        //     esp!(unsafe { esp_hf_client_send_chld_cmd(location as _) })
        // }

        pub fn reply_hold(&self, location: usize) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_send_btrh_cmd(location as _) })
        }

        pub fn dtmf(&self, location: usize) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_send_dtmf(location as _) })
        }

        // pub fn apple(&self, location: usize) -> Result<(), EspError> {
        //     esp!(unsafe { esp_hf_client_send_xapl(location as _) })
        // }

        // pub fn apple_iphone_info(&self, location: usize) -> Result<(), EspError> {
        //     esp!(unsafe { esp_hf_client_send_iphoneaccev(location as _) })
        // }

        pub fn answer(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_answer_call() })
        }

        pub fn reject(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_reject_call() })
        }

        pub fn request_current_calls(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_query_current_calls() })
        }

        pub fn request_current_operator_name(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_query_current_operator_name() })
        }

        pub fn request_subscriber_info(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_retrieve_subscriber_info() })
        }

        pub fn request_last_voice_tag_number(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_request_last_voice_tag_number() })
        }

        pub fn disable_ag_aec(&self) -> Result<(), EspError> {
            esp!(unsafe { esp_hf_client_send_nrec() })
        }

        #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
        pub fn request_outgoing_data_ready(&self) {
            unsafe {
                esp_hf_client_outgoing_data_ready();
            }
        }

        #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
        pub fn pcm_resample(&self, src: &[u8], dst: &mut [u8]) -> Result<usize, EspError> {
            if self.resampling_source.is_some() {
                if dst.len() >= src.len() {
                    Ok(unsafe {
                        esp_hf_client_pcm_resample(
                            src.as_ptr() as *mut ffi::c_void,
                            src.len() as _,
                            dst.as_ptr() as *mut ffi::c_void,
                        )
                    } as _)
                } else {
                    Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>())
                }
            } else {
                Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
            }
        }

        unsafe extern "C" fn event_handler(
            event: esp_hf_client_cb_event_t,
            param: *mut esp_hf_client_cb_param_t,
        ) {
            let param = unsafe { param.as_ref() }.unwrap();
            let event = HfpcEvent::from((event, param));

            info!("Got event {{ {:#?} }}", event);
        }

        #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
        unsafe extern "C" fn recv_data_handler(buf: *const u8, len: u32) {
            let event = HfpcEvent::RecvData(core::slice::from_raw_parts(buf, len as _));
            debug!("Got event {{ {:#?} }}", event);

            CALLBACK.call(event);
        }

        #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
        unsafe extern "C" fn send_data_handler(buf: *mut u8, len: u32) -> u32 {
            let event = HfpcEvent::SendData(core::slice::from_raw_parts_mut(buf, len as _));
            debug!("Got event {{ {:#?} }}", event);

            CALLBACK.call(event) as _
        }
    }

    impl<'d, M, T> Drop for EspHfpc<'d, M, T>
    where
        T: Borrow<BtDriver<'d, M>>,
        M: BtClassicEnabled,
    {
        fn drop(&mut self) {
            if self.initialized.load(Ordering::SeqCst) {
                #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
                if self.resampling_source.is_some() {
                    unsafe {
                        esp_hf_client_pcm_resample_deinit();
                    }
                }

                #[cfg(esp_idf_bt_hfp_audio_data_path_hci)]
                esp!(unsafe { esp_hf_client_register_data_callback(None, None) }).unwrap();

                esp!(unsafe { esp_hf_client_register_callback(None) }).unwrap();
                esp!(unsafe { esp_hf_client_deinit() }).unwrap();

                CALLBACK.clear().unwrap();
            }
        }
    }

    static CALLBACK: BtCallback<HfpcEvent, usize> = BtCallback::new(0);
}
