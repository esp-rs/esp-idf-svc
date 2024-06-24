use crate::sdspi::isr::Service;
use esp_idf_hal::gpio::{InputPin, OutputPin};
use esp_idf_hal::spi::Spi;
use esp_idf_sys::*;
use std::borrow::Borrow;
use std::marker::PhantomData;

pub struct Host(PhantomData<()>);

impl Host {
    pub fn new() -> Result<Self, EspError> {
        esp!(unsafe { sdspi_host_init() })?;
        Ok(Host(PhantomData))
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        esp!(unsafe { sdspi_host_deinit() }).unwrap();
    }
}

pub struct Device<
    H: Borrow<Host>,
    S: Borrow<Service>,
    CS: OutputPin,
    CD: InputPin,
    WP: OutputPin,
    INT: InputPin,
> {
    handle: sdspi_dev_handle_t,
    _host: H,
    _service: S,
    _cs: CS,
    _cd: Option<CD>,
    _wp: Option<WP>,
    _int: Option<INT>,
}

impl<
        H: Borrow<Host>,
        S: Borrow<Service>,
        CS: OutputPin,
        CD: InputPin,
        WP: OutputPin,
        INT: InputPin,
    > Device<H, S, CS, CD, WP, INT>
{
    pub fn new<SPI: Spi>(
        host: H,
        service: S,
        cs: CS,
        cd: Option<CD>,
        wp: Option<WP>,
        int: Option<INT>,
    ) -> Result<Self, EspError> {
        let dev_config = sdspi_device_config_t {
            host_id: SPI::device(),
            gpio_cs: cs.pin(),
            gpio_cd: cd.as_ref().map_or(gpio_num_t_GPIO_NUM_NC, |pin| pin.pin()),
            gpio_wp: wp.as_ref().map_or(gpio_num_t_GPIO_NUM_NC, |pin| pin.pin()),
            gpio_int: int.as_ref().map_or(gpio_num_t_GPIO_NUM_NC, |pin| pin.pin()),
        };

        let mut device_handle: sdspi_dev_handle_t = Default::default();

        esp!(unsafe { sdspi_host_init_device(&dev_config, &mut device_handle as *mut _) })?;

        Ok(Self {
            handle: device_handle,
            _host: host,
            _service: service,
            _cs: cs,
            _cd: cd,
            _wp: wp,
            _int: int,
        })
    }
}

impl<
        H: Borrow<Host>,
        S: Borrow<Service>,
        CS: OutputPin,
        CD: InputPin,
        WP: OutputPin,
        INT: InputPin,
    > Drop for Device<H, S, CS, CD, WP, INT>
{
    fn drop(&mut self) {
        esp!(unsafe { sdspi_host_remove_device(self.handle) }).unwrap();
    }
}

pub struct SdMmcCard(pub(crate) sdmmc_card_t);

impl SdMmcCard {
    pub fn init<
        H: Borrow<Host>,
        S: Borrow<Service>,
        CS: OutputPin,
        CD: InputPin,
        WP: OutputPin,
        INT: InputPin,
    >(
        host: &Device<H, S, CS, CD, WP, INT>,
    ) -> Result<Self, EspError> {
        // https://github.com/espressif/esp-idf/blob/v4.4/components/driver/include/driver/sdmmc_types.h#L137-L139
        #[allow(non_snake_case)]
        let SDMMC_HOST_FLAG_SPI = 1 << 3;
        #[allow(non_snake_case)]
        let SDMMC_HOST_FLAG_DEINIT_ARG = 1 << 5;

        // https://github.com/espressif/esp-idf/blob/v4.4/components/driver/include/driver/sdspi_host.h#L36
        let host_config = sdmmc_host_t {
            flags: SDMMC_HOST_FLAG_SPI | SDMMC_HOST_FLAG_DEINIT_ARG,
            slot: host.handle, // SDSPI_DEFAULT_HOST
            max_freq_khz: SDMMC_FREQ_DEFAULT as _,
            io_voltage: 3.3f32,
            init: Some(sdspi_host_init),
            set_bus_width: None,
            get_bus_width: None,
            set_bus_ddr_mode: None,
            set_card_clk: Some(sdspi_host_set_card_clk),
            do_transaction: Some(sdspi_host_do_transaction),
            __bindgen_anon_1: sdmmc_host_t__bindgen_ty_1 {
                deinit_p: Some(sdspi_host_remove_device),
            },
            io_int_enable: Some(sdspi_host_io_int_enable),
            io_int_wait: Some(sdspi_host_io_int_wait),
            command_timeout_ms: 0,
        };

        let mut card: sdmmc_card_t = Default::default();

        esp!(unsafe { sdmmc_card_init(&host_config, &mut card) })?;

        Ok(SdMmcCard(card))
    }
}

mod isr {
    use esp_idf_sys::*;
    use std::borrow::Borrow;

    pub struct Service;
    pub struct Handler<S: Borrow<Service>, C: FnMut()> {
        _service: S,
        pin: i32,
        callback_ptr: *mut C,
    }

    impl Service {
        pub fn new(intr_alloc_flags: i32) -> Result<Service, EspError> {
            let result = esp!(unsafe { gpio_install_isr_service(intr_alloc_flags) });
            result.map(|_| Service)
        }
    }

    impl Drop for Service {
        fn drop(&mut self) {
            unsafe {
                gpio_uninstall_isr_service();
            }
        }
    }

    impl<S: Borrow<Service>, C: FnMut()> Handler<S, C> {
        pub fn new(service: S, pin: i32, callback: C) -> Result<Self, EspError> {
            let callback = Box::new(callback);
            let callback_ptr = Box::into_raw(callback);

            let result = esp!(unsafe {
                gpio_isr_handler_add(pin, Some(Self::handler), callback_ptr as *mut _)
            });

            match result {
                Ok(_) => Ok(Self {
                    _service: service,
                    pin,
                    callback_ptr,
                }),
                Err(err) => {
                    unsafe {
                        Box::from_raw(callback_ptr);
                    };
                    Err(err)
                }
            }
        }

        unsafe extern "C" fn handler(arg: *mut c_types::c_void) {
            let callback = arg as *mut C;
            (*callback)();
        }
    }

    impl<S: Borrow<Service>, C: FnMut()> Drop for Handler<S, C> {
        fn drop(&mut self) {
            let result = esp!(unsafe { gpio_isr_handler_remove(self.pin) });
            unsafe {
                Box::from_raw(self.callback_ptr);
            }
            result.expect("Failed to remove interrupt handler.");
        }
    }
}
