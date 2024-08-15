use at_commands::{builder::CommandBuilder, parser::CommandParser};
use atat::{self, AtatCmd};
use core::{borrow::BorrowMut, ffi::c_void, marker::PhantomData};
use esp_idf_hal::{
    delay::{TickType, BLOCK},
    uart::{UartDriver, UartTxDriver},
};

use crate::{
    handle::RawHandle,
    netif::{EspNetif, EspNetifDriver, NetifStack},
    sys::*,
};

pub struct EspModem<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    serial: T,

    _d: PhantomData<&'d ()>,
}

impl<'d, T> EspModem<'d, T>
where
    T: BorrowMut<UartDriver<'d>> + Send,
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
        let netif = EspNetif::new(NetifStack::Ppp)?;

        let (mut tx, rx) = self.serial.borrow_mut().split();
        let driver = EspNetifDriver::new_ppp(netif, move |x| Self::tx(&mut tx, x))?;

        let mut buff = [0u8; 64];
        loop {
            let len = rx.read(&mut buff, BLOCK)?;
            if len > 0 {
                driver.rx(&buff)?;
            }
        }

        Ok(())
    }

    fn tx(writer: &mut UartTxDriver, data: &[u8]) -> Result<(), EspError> {
        writer.write(data)?;
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
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
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
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
        Ok(())
    }

    fn set_echo(&mut self, echo: bool) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_execute(&mut buff, false)
            .named(format!("ATE{}", i32::from(echo)))
            .finish()
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
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
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
        Ok(())
    }

    fn read_gprs_registration_status(&mut self) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_query(&mut buff, true)
            .named("+CGREG")
            .finish()
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
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
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
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
            .with_string_parameter("flowlive.net") // apn
            .finish()
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
        self.serial.borrow_mut().write(cmd)?;
        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())?;
        log::info!("got response {:?}", &buff[..len]);

        CommandParser::parse(&buff[..len])
            .expect_identifier(b"\r\nOK\r\n")
            .finish()
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
        Ok(())
    }

    fn set_data_mode(&mut self) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_execute(&mut buff, false)
            .named("ATD*99#")
            .finish()
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
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
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
        log::info!("connect {:?}", connect_parm);
        Ok(())
    }
}
