use at_commands::{builder::CommandBuilder, parser::CommandParser};
use atat::{self, AtatCmd};
use core::{borrow::BorrowMut, ffi::c_void, marker::PhantomData};
use esp_idf_hal::{
    delay::{TickType, BLOCK},
    io::EspIOError,
    uart::{UartDriver, UartTxDriver},
};

use crate::{
    eventloop::EspSystemEventLoop,
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

    pub fn setup_data_mode(&mut self, sysloop: EspSystemEventLoop) -> Result<(), EspError> {
        self.reset()?;
        //disable echo
        self.set_echo(false)?;

        // get iccid
        self.get_iccid()?;

        // check pdp network reg
        self.read_gprs_registration_status()?;

        //configure apn
        self.set_pdp_context()?;

        // start ppp
        self.set_data_mode()?;

        // now in ppp mode.
        let netif = EspNetif::new(NetifStack::Ppp)?;
        // subscribe to user event
        esp!(unsafe {
            esp_event_handler_register(
                IP_EVENT,
                ESP_EVENT_ANY_ID as _,
                Some(Self::raw_on_ip_event),
                netif.handle() as *mut core::ffi::c_void,
            )
        })?;
        esp!(unsafe {
            esp_event_handler_register(
                NETIF_PPP_STATUS,
                ESP_EVENT_ANY_ID as _,
                Some(Self::raw_on_ppp_changed),
                netif.handle() as *mut core::ffi::c_void,
            )
        })?;
        let (mut tx, rx) = self.serial.borrow_mut().split();
        let driver = unsafe {
            EspNetifDriver::new_nonstatic_ppp(&netif, sysloop, move |x| Self::tx(&mut tx, x))?
        };

        let mut buff = [0u8; 64];
        loop {
            let len = rx.read(&mut buff, BLOCK)?;
            if len > 0 {
                driver.rx(&buff[..len])?;
            }
        }

        Ok(())
    }

    fn tx(writer: &mut UartTxDriver, data: &[u8]) -> Result<(), EspError> {
        esp_idf_hal::io::Write::write_all(writer, data).map_err(|w| w.0)?;
        // writer.write_all(data).map_err(|w| w.0)?;
        // writer.write(data)?;
        Ok(())
    }

    fn on_ip_event(event_id: u32, event_data: *mut ::core::ffi::c_void) {
        use log::info;
        info!("Got event id: {}", event_id);

        if event_id == ip_event_t_IP_EVENT_PPP_GOT_IP {
            let mut dns_info = esp_netif_dns_info_t::default();
            let event_data = unsafe { (event_data as *const ip_event_got_ip_t).as_ref() }.unwrap();
            info!(" ip_info = {:?} ", event_data.ip_info);
            info!("modem connected to ppp server, info: {:?}", event_data);

            let netif = event_data.esp_netif;
            esp!(unsafe { esp_netif_get_dns_info(netif, 0, &mut dns_info) }).unwrap();
            info!(" dns_info = {:?} ", unsafe { dns_info.ip.u_addr.ip4.addr });
        } else if event_id == ip_event_t_IP_EVENT_PPP_LOST_IP {
            info!("Modem disconnected from ppp server");
        }
    }

    unsafe extern "C" fn raw_on_ip_event(
        event_handler_arg: *mut ::core::ffi::c_void,
        event_base: esp_event_base_t,
        event_id: i32,
        event_data: *mut ::core::ffi::c_void,
    ) {
        Self::on_ip_event(event_id as _, event_data)
    }

    fn on_ppp_changed(event_id: u32, event_data: *mut ::core::ffi::c_void) {
        use log::info;
        info!("Got event id ppp changed: {}", event_id);

        if event_id == esp_netif_ppp_status_event_t_NETIF_PPP_ERRORUSER {
            info!("user interrupted event from netif");
        }
    }

    unsafe extern "C" fn raw_on_ppp_changed(
        event_handler_arg: *mut ::core::ffi::c_void,
        event_base: esp_event_base_t,
        event_id: i32,
        event_data: *mut ::core::ffi::c_void,
    ) {
        Self::on_ppp_changed(event_id as _, event_data)
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

    fn get_iccid(&mut self) -> Result<(), EspError> {
        let mut buff = [0u8; 64];
        let cmd = CommandBuilder::create_execute(&mut buff, true)
            .named("+CICCID")
            .finish()
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
        self.serial.borrow_mut().write(cmd)?;
        let len = self
            .serial
            .borrow_mut()
            .read(&mut buff, TickType::new_millis(1000).ticks())?;
        log::info!("got response {:?}", &buff[..len]);

        // \r\n+CSQ: 19,99\r\n\r\nOK\r\n
        let (ccid,) = CommandParser::parse(&buff[..len])
            .expect_identifier(b"\r\n+ICCID: ")
            .expect_raw_string()
            .expect_identifier(b"\r\n\r\nOK\r\n")
            .finish()
            .map_err(|_w| EspError::from_infallible::<ESP_FAIL>())?;
        log::info!("ICCID {}", ccid);
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
            .with_string_parameter("flolive.net") // apn
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
