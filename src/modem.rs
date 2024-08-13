use at_commands::{builder::CommandBuilder, parser::CommandParser};
use atat::{self, AtatCmd};
use core::{borrow::BorrowMut, ffi::c_void, marker::PhantomData};
use esp_idf_hal::{delay::TickType, uart::UartDriver};

use crate::{
    handle::RawHandle,
    netif::{EspNetif, NetifStack},
    sys::*,
};

pub struct PppNetif<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    serial: T,
    base: esp_netif_driver_base_t,
    netif: EspNetif,
    _d: PhantomData<&'d ()>,
}

impl<'d, T> PppNetif<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    pub fn new(serial: T) -> Result<Self, EspError> {
        let netif = EspNetif::new(NetifStack::Ppp)?;
        let mut base = esp_netif_driver_base_t {
            netif: netif.handle(),
            post_attach: Some(Self::post_attach),
        };
        let base_ptr: *mut c_void = &mut base as *mut _ as *mut c_void;
        esp!(unsafe { esp_netif_attach(netif.handle(), base_ptr) })?;
        Ok(Self {
            serial,
            netif,
            base,
            _d: PhantomData,
        })
    }

    unsafe extern "C" fn post_attach(netif: *mut esp_netif_obj, args: *mut c_void) -> i32 {
        let driver= unsafe{std::ptr::slice_from_raw_parts_mut(args, size_of::<ppp_netif_driver())}
        let ifconfig = esp_netif_driver_ifconfig_t{handle}
    }
}

pub struct EspModem<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    serial: T,

    _d: PhantomData<&'d ()>,
}

impl<'d, T> EspModem<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    pub fn new(serial: T) -> Self {
        Self {
            serial,
            _d: PhantomData,
        }
    }

    pub fn send_cmd<CMD: AtatCmd>(&mut self, cmd: &CMD) -> Result<CMD::Response, atat::Error> {
        let mut buff = [0u8; 64];
        // flush the channel
        // self.serial
        //     .borrow_mut()
        //     .clear_rx()
        //     .map_err(|_err| atat::Error::Write)?;

        // write the command to the uart
        let len = cmd.write(&mut buff);
        log::info!("about to write {:?}", &buff[..len]);
        self.serial
            .borrow_mut()
            .write(&buff[..len])
            .map_err(|_err| atat::Error::Write)?;

        // now read the uart to get the response

        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())
            .map_err(|_err| atat::Error::Read)?;
        log::info!("got response {:?}", &buff[..len]);
        cmd.parse(Ok(&buff[..len]))
    }

    pub fn setup_data_mode(&mut self) -> Result<(), EspError> {
        self.reset()?;
        //disable echo
        self.set_echo(false)?;

        // check pdp network reg
        self.read_gprs_registration_status()?;

        //configure apn
        self.set_pdp_context()?;

        // start ppp
        self.set_data_mode()?;

        // now in ppp mode.
        // self.netif.

        Ok(())
    }

    fn get_signal_quality(&mut self) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_execute(&mut buff, true)
            .named("+CSQ")
            .finish()
            .unwrap();
        self.serial.borrow_mut().write(cmd)?;
        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())?;
        log::info!("got response {:?}", &buff[..len]);

        // \r\n+CSQ: 19,99\r\n\r\nOK\r\n
        let (rssi, ber) = CommandParser::parse(&buff[..len])
            .expect_identifier(b"\r\n+CSQ: ")
            .expect_int_parameter()
            .expect_int_parameter()
            .expect_identifier(b"\r\n\r\nOK\r\n")
            .finish()
            .unwrap();
        log::info!("Signal Quality: rssi: {} ber: {}", rssi, ber);
        Ok(())
    }

    fn reset(&mut self) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_execute(&mut buff, false)
            .named("ATZ0")
            .finish()
            .unwrap();
        self.serial.borrow_mut().write(cmd)?;
        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())?;
        log::info!("got response {:?}", &buff[..len]);
        CommandParser::parse(&buff[..len])
            .expect_identifier(b"ATZ0\r")
            .expect_identifier(b"\r\nOK\r\n")
            .finish()
            .unwrap();
        Ok(())
    }

    fn set_echo(&mut self, echo: bool) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_execute(&mut buff, false)
            .named(format!("ATE{}", i32::from(echo)))
            .finish()
            .unwrap();
        self.serial.borrow_mut().write(cmd)?;
        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())?;
        log::info!("got response {:?}", &buff[..len]);

        CommandParser::parse(&buff[..len])
            .expect_identifier(b"ATE0\r")
            .expect_identifier(b"\r\nOK\r\n")
            .finish()
            .unwrap();
        Ok(())
    }

    fn read_gprs_registration_status(&mut self) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_query(&mut buff, true)
            .named("+CGREG")
            .finish()
            .unwrap();
        self.serial.borrow_mut().write(cmd)?;
        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())?;
        log::info!("got response {:?}", &buff[..len]);

        let (n, stat, lac, ci) = CommandParser::parse(&buff[..len])
            .expect_identifier(b"\r\n+CGREG: ")
            .expect_int_parameter()
            .expect_int_parameter()
            .expect_optional_int_parameter()
            .expect_optional_int_parameter()
            .expect_identifier(b"\r\n\r\nOK\r\n")
            .finish()
            .unwrap();
        log::info!(
            "CGREG: n: {}stat: {}, lac: {:?}, ci: {:?} ",
            n,
            stat,
            lac,
            ci
        );
        Ok(())
    }

    fn set_pdp_context(&mut self) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_set(&mut buff, true)
            .named("+CGDCONT")
            .with_int_parameter(1) // context id
            .with_string_parameter("IP") // pdp type
            .with_string_parameter("internet")
            .finish()
            .unwrap();
        self.serial.borrow_mut().write(cmd)?;
        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())?;
        log::info!("got response {:?}", &buff[..len]);

        CommandParser::parse(&buff[..len])
            .expect_identifier(b"\r\nOK\r\n")
            .finish()
            .unwrap();

        Ok(())
    }

    fn set_data_mode(&mut self) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_execute(&mut buff, false)
            .named("ATD*99#")
            .finish()
            .unwrap();
        self.serial.borrow_mut().write(cmd)?;
        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())?;
        log::info!("got response {:?}", &buff[..len]);

        let (connect_parm,) = CommandParser::parse(&buff[..len])
            .expect_identifier(b"\r\nCONNECT ")
            .expect_optional_raw_string()
            .expect_identifier(b"\r\n")
            .finish()
            .unwrap();
        log::info!("connect {:?}", connect_parm);
        Ok(())
    }
}
