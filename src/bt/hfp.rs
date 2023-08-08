pub mod client {
    use core::{borrow::Borrow, marker::PhantomData};

    use esp_idf_sys::*;

    use crate::bt::{BtCallback, BtClassicEnabled, BtDriver};

    pub struct HfpcEvent {}

    pub struct EspHfpc<'d, M, T>
    where
        T: Borrow<BtDriver<'d, M>>,
        M: BtClassicEnabled,
    {
        _driver: T,
        _p: PhantomData<&'d ()>,
        _m: PhantomData<M>,
    }

    impl<'d, M, T> EspHfpc<'d, M, T>
    where
        T: Borrow<BtDriver<'d, M>>,
        M: BtClassicEnabled,
    {
        pub fn new<F>(driver: T, events_cb: F) -> Result<Self, EspError>
        where
            F: Fn(&HfpcEvent) + Send + 'static,
        {
            CALLBACK.set(events_cb)?;

            esp!(unsafe { esp_hf_client_init() })?;
            esp!(unsafe { esp_hf_client_register_callback(Some(Self::event_handler)) })?;

            esp!(unsafe {
                esp_hf_client_register_data_callback(
                    Some(Self::recv_data_handler),
                    Some(Self::send_data_handler),
                )
            })?;

            Ok(Self {
                _driver: driver,
                _p: PhantomData,
                _m: PhantomData,
            })
        }

        unsafe extern "C" fn event_handler(
            event: esp_hf_client_cb_event_t,
            param: *mut esp_hf_client_cb_param_t,
        ) {
            let param = unsafe { param.as_ref() }.unwrap();
            //let event = A2dpEvent::from((event, param));

            //debug!("Got GAP event {{ {:#?} }}", &event);
        }

        unsafe extern "C" fn recv_data_handler(buf: *const u8, len: u32) {
            //debug!("Got GAP event {{ {:#?} }}", &event);
            panic!()
        }

        unsafe extern "C" fn send_data_handler(buf: *mut u8, len: u32) -> u32 {
            //debug!("Got GAP event {{ {:#?} }}", &event);
            panic!()
        }
    }

    impl<'d, M, T> Drop for EspHfpc<'d, M, T>
    where
        T: Borrow<BtDriver<'d, M>>,
        M: BtClassicEnabled,
    {
        fn drop(&mut self) {
            esp!(unsafe { esp_hf_client_register_data_callback(None, None) }).unwrap();
            esp!(unsafe { esp_hf_client_register_callback(None) }).unwrap();
            esp!(unsafe { esp_hf_client_deinit() }).unwrap();

            CALLBACK.clear().unwrap();
        }
    }

    static CALLBACK: BtCallback<&HfpcEvent> = BtCallback::new();
}
