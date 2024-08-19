use core::{borrow::BorrowMut, marker::PhantomData};
use esp_idf_hal::{
    delay::BLOCK,
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
    sysloop: EspSystemEventLoop,
    netif: EspNetif,
    running: bool,
    _d: PhantomData<&'d ()>,
}

impl<'d, T> EspModem<'d, T>
where
    T: BorrowMut<UartDriver<'d>> + Send,
{
    pub fn new(serial: T, sysloop: EspSystemEventLoop) -> Result<Self, EspError> {
        Ok(Self {
            serial,
            sysloop,
            netif: EspNetif::new(NetifStack::Ppp)?,
            running: false,
            _d: PhantomData,
        })
    }

    pub fn run(&mut self) -> Result<(), EspError> {
        self.running = true;
        // now in ppp mode.
        // let netif = EspNetif::new(NetifStack::Ppp)?;

        // subscribe to user event
        esp!(unsafe {
            esp_event_handler_register(
                IP_EVENT,
                ESP_EVENT_ANY_ID as _,
                Some(Self::raw_on_ip_event),
                self.netif.handle() as *mut core::ffi::c_void,
            )
        })?;
        esp!(unsafe {
            esp_event_handler_register(
                NETIF_PPP_STATUS,
                ESP_EVENT_ANY_ID as _,
                Some(Self::raw_on_ppp_changed),
                self.netif.handle() as *mut core::ffi::c_void,
            )
        })?;
        let (mut tx, rx) = self.serial.borrow_mut().split();
        let driver = unsafe {
            EspNetifDriver::new_nonstatic_ppp(&self.netif, move |x| Self::tx(&mut tx, x))?
        };

        let mut buff = [0u8; 64];
        loop {
            if !self.running {
                break;
            }
            let len = rx.read(&mut buff, BLOCK)?;
            if len > 0 {
                driver.rx(&buff[..len])?;
            }
        }

        Ok(())
    }

    pub fn is_connected(&self) -> Result<bool, EspError> {
        self.netif.is_up()
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
}

pub mod modem {
    use esp_idf_hal::uart::UartDriver;

    /// Models a modem and enables

    pub enum CommunicationMode {
        Command,
        Data,
    }

    #[derive(Debug)]
    pub enum ModemError {
        IO,
        ATParse(at_commands::parser::ParseError),
        ATBuild(usize),
    }

    impl std::fmt::Display for ModemError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "{:?}", self)
        }
    }

    impl std::error::Error for ModemError {}

    pub trait SimModem {
        /// get the current mode of the sim modem
        fn get_mode(&self) -> &CommunicationMode;

        /// Initialise the remote modem so that it is in PPPoS mode.
        fn negotiate(&mut self, comm: &mut UartDriver) -> Result<(), ModemError>;
    }

    impl From<usize> for ModemError {
        fn from(value: usize) -> Self {
            ModemError::ATBuild(value)
        }
    }

    impl From<at_commands::parser::ParseError> for ModemError {
        fn from(value: at_commands::parser::ParseError) -> Self {
            ModemError::ATParse(value)
        }
    }

    pub mod sim7600 {

        use core::time::Duration;

        use at_commands::{builder::CommandBuilder, parser::CommandParser};
        use esp_idf_hal::{delay::TickType, uart::UartDriver};

        use super::{CommunicationMode, ModemError, SimModem};
        pub struct SIM7600(CommunicationMode);

        impl SIM7600 {
            pub fn new() -> Self {
                Self(CommunicationMode::Command)
            }
        }

        pub fn get_signal_quality<IO: embedded_svc::io::Read + embedded_svc::io::Write>(
            comm: &mut IO,
        ) -> Result<(i32, i32), ModemError> {
            let mut buff = [0u8; 64];
            let cmd = CommandBuilder::create_execute(&mut buff, true)
                .named("+CSQ")
                .finish()?;

            comm.write(cmd).map_err(|_| ModemError::IO)?;

            let len = comm.read(&mut buff).map_err(|_| ModemError::IO)?;

            log::info!("got response {:?}", &buff[..len]);

            // \r\n+CSQ: 19,99\r\n\r\nOK\r\n
            Ok(CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\n+CSQ: ")
                .expect_int_parameter()
                .expect_int_parameter()
                .expect_identifier(b"\r\n\r\nOK\r\n")
                .finish()?)
        }

        fn get_iccid<IO: embedded_svc::io::Read + embedded_svc::io::Write>(
            comm: &mut IO,
        ) -> Result<heapless::String<22>, ModemError> {
            let mut buff = [0u8; 64];
            let cmd = CommandBuilder::create_execute(&mut buff, true)
                .named("+CICCID")
                .finish()?;

            comm.write(cmd).map_err(|_| ModemError::IO)?;

            let len = comm.read(&mut buff).map_err(|_| ModemError::IO)?;
            log::info!("got response {:?}", &buff[..len]);

            let (ccid,) = CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\n+ICCID: ")
                .expect_raw_string()
                .expect_identifier(b"\r\n\r\nOK\r\n")
                .finish()?;

            Ok(heapless::String::try_from(ccid).unwrap())
        }

        fn reset(comm: &mut UartDriver) -> Result<(), ModemError> {
            let mut buff = [0u8; 64];
            let cmd = CommandBuilder::create_execute(&mut buff, false)
                .named("ATZ0")
                .finish()?;
            log::info!("cmd={:?}", cmd);

            comm.write(cmd).map_err(|_| ModemError::IO)?;
            log::info!("wrote all");
            std::thread::sleep(Duration::from_secs(1));

            let len = comm
                .read(&mut buff, TickType::new_millis(1000).ticks())
                .map_err(|_| ModemError::IO)?;
            log::info!("len = {}", len);
            log::info!("got response {:?}", &buff[..len]);
            CommandParser::parse(&buff[..len])
                .expect_identifier(b"ATZ0\r")
                .expect_identifier(b"\r\nOK\r\n")
                .finish()?;
            Ok(())
        }

        fn set_echo<IO: embedded_svc::io::Read + embedded_svc::io::Write>(
            comm: &mut IO,
            echo: bool,
        ) -> Result<(), ModemError> {
            let mut buff = [0u8; 64];
            let cmd = CommandBuilder::create_execute(&mut buff, false)
                .named(format!("ATE{}", i32::from(echo)))
                .finish()?;
            comm.write_all(cmd).map_err(|_| ModemError::IO)?;
            let len = comm.read(&mut buff).map_err(|_| ModemError::IO)?;
            log::info!("got response {:?}", &buff[..len]);

            CommandParser::parse(&buff[..len])
                .expect_identifier(b"ATE0\r")
                .expect_identifier(b"\r\nOK\r\n")
                .finish()?;
            Ok(())
        }

        fn read_gprs_registration_status<IO: embedded_svc::io::Read + embedded_svc::io::Write>(
            comm: &mut IO,
        ) -> Result<(i32, i32, Option<i32>, Option<i32>), ModemError> {
            let mut buff = [0u8; 64];
            let cmd = CommandBuilder::create_query(&mut buff, true)
                .named("+CGREG")
                .finish()?;
            comm.write_all(cmd).map_err(|_| ModemError::IO)?;
            let len = comm.read(&mut buff).map_err(|_| ModemError::IO)?;
            log::info!("got response {:?}", &buff[..len]);

            Ok(CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\n+CGREG: ")
                .expect_int_parameter()
                .expect_int_parameter()
                .expect_optional_int_parameter()
                .expect_optional_int_parameter()
                .expect_identifier(b"\r\n\r\nOK\r\n")
                .finish()?)
        }

        fn set_pdp_context<IO: embedded_svc::io::Read + embedded_svc::io::Write>(
            comm: &mut IO,
        ) -> Result<(), ModemError> {
            let mut buff = [0u8; 64];
            let cmd = CommandBuilder::create_set(&mut buff, true)
                .named("+CGDCONT")
                .with_int_parameter(1) // context id
                .with_string_parameter("IP") // pdp type
                .with_string_parameter("flolive.net") // apn
                .finish()?;
            comm.write_all(cmd).map_err(|_| ModemError::IO)?;
            let len = comm.read(&mut buff).map_err(|_| ModemError::IO)?;
            log::info!("got response {:?}", &buff[..len]);

            CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\nOK\r\n")
                .finish()?;
            Ok(())
        }

        fn set_data_mode<IO: embedded_svc::io::Read + embedded_svc::io::Write>(
            comm: &mut IO,
        ) -> Result<(), ModemError> {
            let mut buff = [0u8; 64];
            let cmd = CommandBuilder::create_execute(&mut buff, false)
                .named("ATD*99#")
                .finish()?;
            comm.write_all(cmd).map_err(|_| ModemError::IO)?;
            let len = comm.read(&mut buff).map_err(|_| ModemError::IO)?;
            log::info!("got response {:?}", &buff[..len]);

            let (connect_parm,) = CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\nCONNECT ")
                .expect_optional_raw_string()
                .expect_identifier(b"\r\n")
                .finish()?;
            log::info!("connect {:?}", connect_parm);
            Ok(())
        }

        impl SimModem for SIM7600 {
            fn negotiate(&mut self, comm: &mut UartDriver) -> Result<(), ModemError> {
                reset(comm)?;
                //disable echo
                set_echo(comm, false)?;

                // get iccid
                get_iccid(comm)?;

                // check pdp network reg
                read_gprs_registration_status(comm)?;

                //configure apn
                set_pdp_context(comm)?;

                // start ppp
                set_data_mode(comm)?;

                self.0 = CommunicationMode::Data;
                Ok(())
            }

            fn get_mode(&self) -> &CommunicationMode {
                &self.0
            }
        }
    }
}
