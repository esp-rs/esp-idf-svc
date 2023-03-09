use core::fmt::Debug;
use core::marker::PhantomData;
use core::time::Duration;
use core::{ffi, ptr};

use ::log::*;

extern crate alloc;
use alloc::boxed::Box;
use alloc::sync::Arc;

use embedded_svc::eth::*;

use esp_idf_hal::peripheral::Peripheral;

#[cfg(any(
    all(esp32, esp_idf_eth_use_esp32_emac),
    any(
        esp_idf_eth_spi_ethernet_dm9051,
        esp_idf_eth_spi_ethernet_w5500,
        esp_idf_eth_spi_ethernet_ksz8851snl
    )
))]
use esp_idf_hal::gpio;
#[cfg(any(
    esp_idf_eth_spi_ethernet_dm9051,
    esp_idf_eth_spi_ethernet_w5500,
    esp_idf_eth_spi_ethernet_ksz8851snl
))]
use esp_idf_hal::{spi, units::Hertz};

use esp_idf_sys::*;

use crate::eventloop::{
    EspEventLoop, EspSubscription, EspSystemEventLoop, EspTypedEventDeserializer,
    EspTypedEventSource, System,
};
use crate::handle::RawHandle;
#[cfg(esp_idf_comp_esp_netif_enabled)]
use crate::netif::*;
use crate::private::waitable::*;
use crate::private::*;

#[cfg(all(esp32, esp_idf_eth_use_esp32_emac))]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RmiiEthChipset {
    IP101,
    RTL8201,
    LAN87XX,
    DP83848,
    #[cfg(esp_idf_version_major = "4")]
    KSZ8041,
    #[cfg(esp_idf_version = "4.4")]
    KSZ8081,
    #[cfg(not(esp_idf_version_major = "4"))]
    KSZ80XX,
}

#[cfg(all(esp32, esp_idf_eth_use_esp32_emac))]
pub enum RmiiClockConfig<GPIO0, GPIO16, GPIO17>
where
    GPIO0: Peripheral<P = gpio::Gpio0>,
    GPIO16: Peripheral<P = gpio::Gpio16>,
    GPIO17: Peripheral<P = gpio::Gpio17>,
{
    Input(GPIO0),
    #[cfg(not(esp_idf_version = "4.3"))]
    OutputGpio0(GPIO0),
    /// This according to ESP-IDF is for "testing" only
    #[cfg(not(esp_idf_version = "4.3"))]
    OutputGpio16(GPIO16),
    #[cfg(not(esp_idf_version = "4.3"))]
    OutputInvertedGpio17(GPIO17),
}

#[cfg(all(esp32, esp_idf_eth_use_esp32_emac, not(esp_idf_version = "4.3")))]
impl<GPIO0, GPIO16, GPIO17> RmiiClockConfig<GPIO0, GPIO16, GPIO17>
where
    GPIO0: Peripheral<P = gpio::Gpio0>,
    GPIO16: Peripheral<P = gpio::Gpio16>,
    GPIO17: Peripheral<P = gpio::Gpio17>,
{
    fn eth_mac_clock_config(&self) -> eth_mac_clock_config_t {
        let rmii = match self {
            Self::Input(_) => eth_mac_clock_config_t__bindgen_ty_2 {
                clock_mode: emac_rmii_clock_mode_t_EMAC_CLK_EXT_IN,
                clock_gpio: emac_rmii_clock_gpio_t_EMAC_CLK_IN_GPIO,
            },
            Self::OutputGpio0(_) => eth_mac_clock_config_t__bindgen_ty_2 {
                clock_mode: emac_rmii_clock_mode_t_EMAC_CLK_OUT,
                clock_gpio: emac_rmii_clock_gpio_t_EMAC_APPL_CLK_OUT_GPIO,
            },
            Self::OutputGpio16(_) => eth_mac_clock_config_t__bindgen_ty_2 {
                clock_mode: emac_rmii_clock_mode_t_EMAC_CLK_OUT,
                clock_gpio: emac_rmii_clock_gpio_t_EMAC_CLK_OUT_GPIO,
            },
            Self::OutputInvertedGpio17(_) => eth_mac_clock_config_t__bindgen_ty_2 {
                clock_mode: emac_rmii_clock_mode_t_EMAC_CLK_OUT,
                clock_gpio: emac_rmii_clock_gpio_t_EMAC_CLK_OUT_180_GPIO,
            },
        };

        eth_mac_clock_config_t { rmii }
    }
}

#[cfg(any(
    esp_idf_eth_spi_ethernet_dm9051,
    esp_idf_eth_spi_ethernet_w5500,
    esp_idf_eth_spi_ethernet_ksz8851snl
))]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SpiEthChipset {
    #[cfg(esp_idf_eth_spi_ethernet_dm9051)]
    DM9051,
    #[cfg(esp_idf_eth_spi_ethernet_w5500)]
    W5500,
    #[cfg(esp_idf_eth_spi_ethernet_ksz8851snl)]
    KSZ8851SNL,
}

struct RawHandleImpl(esp_eth_handle_t);

unsafe impl Send for RawHandleImpl {}

type RawCallback = Box<dyn FnMut(&[u8]) + 'static>;

struct UnsafeCallback(*mut RawCallback);

impl UnsafeCallback {
    #[allow(clippy::type_complexity)]
    fn from(boxed: &mut Box<RawCallback>) -> Self {
        Self(boxed.as_mut())
    }

    unsafe fn from_ptr(ptr: *mut ffi::c_void) -> Self {
        Self(ptr.cast())
    }

    fn as_ptr(&self) -> *mut ffi::c_void {
        self.0.cast()
    }

    unsafe fn call(&self, data: &[u8]) {
        let reference = self.0.as_mut().unwrap();

        (reference)(data);
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Status {
    Stopped,
    Started,
    Connected,
    Disconnected,
}

pub struct EthDriver<'d> {
    spi_bus: Option<spi_host_device_t>,
    spi_device: Option<spi_device_handle_t>,
    handle: esp_eth_handle_t,
    status: Arc<mutex::Mutex<Status>>,
    _subscription: EspSubscription<System>,
    callback: Option<Box<RawCallback>>,
    _p: PhantomData<&'d mut ()>,
}

#[cfg(all(esp32, esp_idf_eth_use_esp32_emac))]
impl<'d> EthDriver<'d> {
    #[allow(clippy::too_many_arguments)]
    pub fn new_rmii(
        _mac: impl Peripheral<P = esp_idf_hal::mac::MAC> + 'd,
        _rmii_rdx0: impl Peripheral<P = gpio::Gpio25> + 'd,
        _rmii_rdx1: impl Peripheral<P = gpio::Gpio26> + 'd,
        _rmii_crs_dv: impl Peripheral<P = gpio::Gpio27> + 'd,
        rmii_mdc: impl Peripheral<P = impl gpio::OutputPin> + 'd,
        _rmii_txd1: impl Peripheral<P = gpio::Gpio22> + 'd,
        _rmii_tx_en: impl Peripheral<P = gpio::Gpio21> + 'd,
        _rmii_txd0: impl Peripheral<P = gpio::Gpio19> + 'd,
        rmii_mdio: impl Peripheral<P = impl gpio::InputPin + gpio::OutputPin> + 'd,
        rmii_ref_clk_config: RmiiClockConfig<
            impl Peripheral<P = gpio::Gpio0> + 'd,
            impl Peripheral<P = gpio::Gpio16> + 'd,
            impl Peripheral<P = gpio::Gpio17> + 'd,
        >,
        rst: Option<impl Peripheral<P = impl gpio::OutputPin> + 'd>,
        chipset: RmiiEthChipset,
        phy_addr: Option<u32>,
        sysloop: EspSystemEventLoop,
    ) -> Result<Self, EspError> {
        esp_idf_hal::into_ref!(rmii_mdc, rmii_mdio);

        let rst = rst.map(|rst| rst.into_ref().pin());

        let eth = Self::init(
            Self::rmii_mac(rmii_mdc.pin(), rmii_mdio.pin(), &rmii_ref_clk_config),
            Self::rmii_phy(chipset, rst, phy_addr)?,
            None,
            None,
            None,
            sysloop,
        )?;

        Ok(eth)
    }

    fn rmii_phy(
        chipset: RmiiEthChipset,
        reset: Option<i32>,
        phy_addr: Option<u32>,
    ) -> Result<*mut esp_eth_phy_t, EspError> {
        let phy_cfg = Self::eth_phy_default_config(reset, phy_addr);

        let phy = match chipset {
            RmiiEthChipset::IP101 => unsafe { esp_eth_phy_new_ip101(&phy_cfg) },
            RmiiEthChipset::RTL8201 => unsafe { esp_eth_phy_new_rtl8201(&phy_cfg) },
            #[cfg(not(esp_idf_version = "4.3"))]
            RmiiEthChipset::LAN87XX => unsafe { esp_eth_phy_new_lan87xx(&phy_cfg) },
            #[cfg(esp_idf_version = "4.3")]
            RmiiEthChipset::LAN87XX => unsafe { esp_eth_phy_new_lan8720(&phy_cfg) },
            RmiiEthChipset::DP83848 => unsafe { esp_eth_phy_new_dp83848(&phy_cfg) },
            #[cfg(esp_idf_version_major = "4")]
            RmiiEthChipset::KSZ8041 => unsafe { esp_eth_phy_new_ksz8041(&phy_cfg) },
            #[cfg(esp_idf_version = "4.4")]
            RmiiEthChipset::KSZ8081 => unsafe { esp_eth_phy_new_ksz8081(&phy_cfg) },
            #[cfg(not(esp_idf_version_major = "4"))]
            RmiiEthChipset::KSZ80XX => unsafe { esp_eth_phy_new_ksz80xx(&phy_cfg) },
        };

        Ok(phy)
    }

    fn rmii_mac(
        mdc: i32,
        mdio: i32,
        clk_config: &RmiiClockConfig<
            impl Peripheral<P = gpio::Gpio0> + 'd,
            impl Peripheral<P = gpio::Gpio16> + 'd,
            impl Peripheral<P = gpio::Gpio17> + 'd,
        >,
    ) -> *mut esp_eth_mac_t {
        #[cfg(esp_idf_version_major = "4")]
        let mac = {
            let mut config = Self::eth_mac_default_config(mdc, mdio);

            #[cfg(not(esp_idf_version = "4.3"))]
            {
                config.clock_config = clk_config.eth_mac_clock_config();
            }

            unsafe { esp_eth_mac_new_esp32(&config) }
        };

        #[cfg(not(esp_idf_version_major = "4"))]
        let mac = {
            let mut esp32_config = Self::eth_esp32_emac_default_config(mdc, mdio);
            esp32_config.clock_config = clk_config.eth_mac_clock_config();

            let config = Self::eth_mac_default_config(mdc, mdio);

            unsafe { esp_eth_mac_new_esp32(&esp32_config, &config) }
        };

        mac
    }
}

#[cfg(esp_idf_eth_use_openeth)]
impl<'d> EthDriver<'d> {
    pub fn new_openeth(
        _mac: impl Peripheral<P = esp_idf_hal::mac::MAC> + 'd,
        sysloop: EspSystemEventLoop,
    ) -> Result<Self, EspError> {
        let eth = Self::init(
            unsafe { esp_eth_mac_new_openeth(&Self::eth_mac_default_config(0, 0)) },
            unsafe { esp_eth_phy_new_dp83848(&Self::eth_phy_default_config(None, None)) },
            None,
            None,
            None,
            sysloop,
        )?;

        Ok(eth)
    }
}

#[cfg(any(
    esp_idf_eth_spi_ethernet_dm9051,
    esp_idf_eth_spi_ethernet_w5500,
    esp_idf_eth_spi_ethernet_ksz8851snl
))]
impl<'d> EthDriver<'d> {
    pub fn new_spi(
        spi_driver: &spi::SpiDriver<'d>,
        int: impl Peripheral<P = impl gpio::InputPin> + 'd,
        cs: Option<impl Peripheral<P = impl gpio::OutputPin> + 'd>,
        rst: Option<impl Peripheral<P = impl gpio::OutputPin> + 'd>,
        chipset: SpiEthChipset,
        baudrate: Hertz,
        mac_addr: Option<&[u8; 6]>,
        phy_addr: Option<u32>,
        sysloop: EspSystemEventLoop,
    ) -> Result<Self, EspError> {
        esp_idf_hal::into_ref!(int);

        let (mac, phy, spi_device) = Self::init_spi(
            spi_driver.host(),
            chipset,
            baudrate,
            int.pin(),
            cs.map(|pin| pin.into_ref().pin()),
            rst.map(|pin| pin.into_ref().pin()),
            phy_addr,
        )?;

        let eth = Self::init(
            mac,
            phy,
            mac_addr,
            Some(spi_driver.host()),
            spi_device,
            sysloop,
        )?;

        Ok(eth)
    }

    fn init_spi(
        host: spi_host_device_t,
        chipset: SpiEthChipset,
        baudrate: Hertz,
        int: i32,
        cs: Option<i32>,
        rst: Option<i32>,
        phy_addr: Option<u32>,
    ) -> Result<
        (
            *mut esp_eth_mac_t,
            *mut esp_eth_phy_t,
            Option<spi_device_handle_t>,
        ),
        EspError,
    > {
        unsafe { gpio_install_isr_service(0) };

        let mac_cfg = EthDriver::eth_mac_default_config(0, 0);
        let phy_cfg = EthDriver::eth_phy_default_config(rst.map(|pin| pin), phy_addr);

        let (mac, phy, spi_handle) = match chipset {
            #[cfg(esp_idf_eth_spi_ethernet_dm9051)]
            SpiEthChipset::DM9051 => {
                let spi_devcfg = Self::get_spi_conf(cs, 1, 7, baudrate);

                #[cfg(esp_idf_version_major = "4")]
                let spi_handle = Some(Self::init_spi_device(host, &spi_devcfg)?);

                #[cfg(not(esp_idf_version_major = "4"))]
                let spi_handle = None;

                #[cfg(esp_idf_version_major = "4")]
                let dm9051_cfg = eth_dm9051_config_t {
                    spi_hdl: spi_handle.unwrap() as *mut _,
                    int_gpio_num: int,
                };

                #[cfg(not(esp_idf_version_major = "4"))]
                let dm9051_cfg = eth_dm9051_config_t {
                    spi_host_id: P::device() as _,
                    spi_devcfg: &spi_devcfg as *const _ as *mut _,
                    int_gpio_num: int,
                };

                let mac = unsafe { esp_eth_mac_new_dm9051(&dm9051_cfg, &mac_cfg) };
                let phy = unsafe { esp_eth_phy_new_dm9051(&phy_cfg) };

                (mac, phy, spi_handle)
            }
            #[cfg(esp_idf_eth_spi_ethernet_w5500)]
            SpiEthChipset::W5500 => {
                let spi_devcfg = Self::get_spi_conf(cs, 16, 8, baudrate);

                #[cfg(esp_idf_version_major = "4")]
                let spi_handle = Some(Self::init_spi_device(host, &spi_devcfg)?);

                #[cfg(not(esp_idf_version_major = "4"))]
                let spi_handle = None;

                #[cfg(esp_idf_version_major = "4")]
                let w5500_cfg = eth_w5500_config_t {
                    spi_hdl: spi_handle.unwrap() as *mut _,
                    int_gpio_num: int,
                };

                #[cfg(not(esp_idf_version_major = "4"))]
                let w5500_cfg = eth_w5500_config_t {
                    spi_host_id: P::device() as _,
                    spi_devcfg: &spi_devcfg as *const _ as *mut _,
                    int_gpio_num: int,
                };

                let mac = unsafe { esp_eth_mac_new_w5500(&w5500_cfg, &mac_cfg) };
                let phy = unsafe { esp_eth_phy_new_w5500(&phy_cfg) };

                (mac, phy, spi_handle)
            }
            #[cfg(esp_idf_eth_spi_ethernet_ksz8851snl)]
            SpiEthChipset::KSZ8851SNL => {
                let spi_devcfg = Self::get_spi_conf(cs, 0, 0, baudrate);

                #[cfg(esp_idf_version_major = "4")]
                let spi_handle = Some(Self::init_spi_device(host, &spi_devcfg)?);

                #[cfg(not(esp_idf_version_major = "4"))]
                let spi_handle = None;

                #[cfg(esp_idf_version_major = "4")]
                let ksz8851snl_cfg = eth_ksz8851snl_config_t {
                    spi_hdl: spi_handle.unwrap() as *mut _,
                    int_gpio_num: int,
                };

                #[cfg(not(esp_idf_version_major = "4"))]
                let ksz8851snl_cfg = eth_ksz8851snl_config_t {
                    spi_host_id: P::device() as _,
                    spi_devcfg: &spi_devcfg as *const _ as *mut _,
                    int_gpio_num: int,
                };

                let mac = unsafe { esp_eth_mac_new_ksz8851snl(&ksz8851snl_cfg, &mac_cfg) };
                let phy = unsafe { esp_eth_phy_new_ksz8851snl(&phy_cfg) };

                (mac, phy, spi_handle)
            }
        };

        Ok((mac, phy, spi_handle))
    }

    fn get_spi_conf(
        cs: Option<i32>,
        command_bits: u8,
        address_bits: u8,
        baudrate: Hertz,
    ) -> spi_device_interface_config_t {
        spi_device_interface_config_t {
            command_bits,
            address_bits,
            mode: 0,
            clock_speed_hz: baudrate.0 as i32,
            spics_io_num: cs.map(|pin| pin).unwrap_or(-1),
            queue_size: 20,
            ..Default::default()
        }
    }

    #[cfg(esp_idf_version_major = "4")]
    fn init_spi_device(
        host: spi_host_device_t,
        conf: &spi_device_interface_config_t,
    ) -> Result<spi_device_handle_t, EspError> {
        let mut spi_handle: spi_device_handle_t = ptr::null_mut();

        esp!(unsafe { spi_bus_add_device(host, conf, &mut spi_handle) })?;

        Ok(spi_handle)
    }
}

impl<'d> EthDriver<'d> {
    fn init(
        mac: *mut esp_eth_mac_t,
        phy: *mut esp_eth_phy_t,
        mac_addr: Option<&[u8; 6]>,
        spi_bus: Option<spi_host_device_t>,
        spi_device: Option<spi_device_handle_t>,
        sysloop: EspSystemEventLoop,
    ) -> Result<Self, EspError> {
        let cfg = Self::eth_default_config(mac, phy);

        let mut handle: esp_eth_handle_t = ptr::null_mut();
        esp!(unsafe { esp_eth_driver_install(&cfg, &mut handle) })?;

        info!("Driver initialized");

        if let Some(mac_addr) = mac_addr {
            esp!(unsafe {
                esp_eth_ioctl(
                    handle,
                    esp_eth_io_cmd_t_ETH_CMD_S_MAC_ADDR,
                    mac_addr.as_ptr() as *mut _,
                )
            })?;

            info!("Attached MAC address: {:?}", mac_addr);
        }

        let (waitable, subscription) = Self::subscribe(handle, &sysloop)?;

        let eth = Self {
            handle,
            spi_bus,
            spi_device,
            status: waitable,
            _subscription: subscription,
            callback: None,
            _p: PhantomData,
        };

        info!("Initialization complete");

        Ok(eth)
    }

    fn subscribe(
        handle: esp_eth_handle_t,
        sysloop: &EspEventLoop<System>,
    ) -> Result<(Arc<mutex::Mutex<Status>>, EspSubscription<System>), EspError> {
        let status = Arc::new(mutex::Mutex::wrap(mutex::RawMutex::new(), Status::Stopped));
        let s_status = status.clone();

        let handle = RawHandleImpl(handle);

        let subscription = sysloop.subscribe(move |event: &EthEvent| {
            if event.is_for_handle(handle.0) {
                let mut guard = s_status.lock();

                match event {
                    EthEvent::Started(_) => *guard = Status::Started,
                    EthEvent::Stopped(_) => *guard = Status::Stopped,
                    EthEvent::Connected(_) => *guard = Status::Connected,
                    EthEvent::Disconnected(_) => *guard = Status::Disconnected,
                }
            }
        })?;

        Ok((status, subscription))
    }

    pub fn is_started(&self) -> Result<bool, EspError> {
        let guard = self.status.lock();

        Ok(*guard == Status::Started
            || *guard == Status::Connected
            || *guard == Status::Disconnected)
    }

    pub fn is_up(&self) -> Result<bool, EspError> {
        let guard = self.status.lock();

        Ok(*guard == Status::Connected)
    }

    pub fn start(&mut self) -> Result<(), EspError> {
        esp!(unsafe { esp_eth_start(self.handle) })?;

        info!("Start requested");

        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), EspError> {
        info!("Stopping");

        let err = unsafe { esp_eth_stop(self.handle) };
        if err != ESP_ERR_INVALID_STATE {
            esp!(err)?;
        }

        info!("Stop requested");

        Ok(())
    }

    pub fn set_rx_callback<C>(&mut self, mut callback: C) -> Result<(), EspError>
    where
        C: for<'a> FnMut(&[u8]) + Send + 'static,
    {
        let _ = self.stop();

        let mut callback: Box<RawCallback> = Box::new(Box::new(move |data| callback(data)));

        let unsafe_callback = UnsafeCallback::from(&mut callback);

        unsafe {
            esp_eth_update_input_path(self.handle(), Some(Self::handle), unsafe_callback.as_ptr());
        }

        self.callback = Some(callback);

        Ok(())
    }

    pub fn send(&mut self, frame: &[u8]) -> Result<(), EspError> {
        esp!(unsafe {
            esp_eth_transmit(self.handle(), frame.as_ptr() as *mut _, frame.len() as _)
        })?;

        Ok(())
    }

    unsafe extern "C" fn handle(
        _handle: esp_eth_handle_t,
        buf: *mut u8,
        len: u32,
        event_handler_arg: *mut ffi::c_void,
    ) -> esp_err_t {
        UnsafeCallback::from_ptr(event_handler_arg as *mut _)
            .call(core::slice::from_raw_parts(buf, len as _));

        ESP_OK
    }

    fn clear_all(&mut self) -> Result<(), EspError> {
        let _ = self.stop(); // Driver might be stopped already

        unsafe {
            esp!(esp_eth_driver_uninstall(self.handle))?;
        }

        if let Some(device) = self.spi_device {
            esp!(unsafe { spi_bus_remove_device(device) }).unwrap();
        }

        if let Some(bus) = self.spi_bus {
            esp!(unsafe { spi_bus_free(bus) }).unwrap();
        }

        info!("Driver deinitialized");

        Ok(())
    }

    fn eth_default_config(mac: *mut esp_eth_mac_t, phy: *mut esp_eth_phy_t) -> esp_eth_config_t {
        esp_eth_config_t {
            mac,
            phy,
            check_link_period_ms: 2000,
            ..Default::default()
        }
    }

    fn eth_phy_default_config(reset_pin: Option<i32>, phy_addr: Option<u32>) -> eth_phy_config_t {
        eth_phy_config_t {
            phy_addr: phy_addr.map(|a| a as i32).unwrap_or(ESP_ETH_PHY_ADDR_AUTO),
            reset_timeout_ms: 100,
            autonego_timeout_ms: 4000,
            reset_gpio_num: reset_pin.unwrap_or(-1),
        }
    }

    #[cfg(esp_idf_version_major = "4")]
    fn eth_mac_default_config(mdc: i32, mdio: i32) -> eth_mac_config_t {
        eth_mac_config_t {
            sw_reset_timeout_ms: 100,
            rx_task_stack_size: 2048,
            rx_task_prio: 15,
            smi_mdc_gpio_num: mdc,
            smi_mdio_gpio_num: mdio,
            flags: 0,
            #[cfg(esp_idf_version = "4.4")]
            interface: eth_data_interface_t_EMAC_DATA_INTERFACE_RMII,
            ..Default::default()
        }
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    fn eth_mac_default_config(_mdc: i32, _mdio: i32) -> eth_mac_config_t {
        eth_mac_config_t {
            sw_reset_timeout_ms: 100,
            rx_task_stack_size: 2048,
            rx_task_prio: 15,
            flags: 0,
        }
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    fn eth_esp32_emac_default_config(mdc: i32, mdio: i32) -> eth_esp32_emac_config_t {
        eth_esp32_emac_config_t {
            smi_mdc_gpio_num: mdc,
            smi_mdio_gpio_num: mdio,
            interface: eth_data_interface_t_EMAC_DATA_INTERFACE_RMII,
            ..Default::default()
        }
    }
}

impl<'d> Eth for EthDriver<'d> {
    type Error = EspError;

    fn start(&mut self) -> Result<(), Self::Error> {
        EthDriver::start(self)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        EthDriver::stop(self)
    }

    fn is_started(&self) -> Result<bool, Self::Error> {
        EthDriver::is_started(self)
    }

    fn is_up(&self) -> Result<bool, Self::Error> {
        EthDriver::is_up(self)
    }
}

unsafe impl<'d> Send for EthDriver<'d> {}

impl<'d> Drop for EthDriver<'d> {
    fn drop(&mut self) {
        self.clear_all().unwrap();

        info!("Dropped");
    }
}

impl<'d> RawHandle for EthDriver<'d> {
    type Handle = esp_eth_handle_t;

    fn handle(&self) -> Self::Handle {
        self.handle
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
pub struct EspEth<'d> {
    glue_handle: *mut esp_eth_netif_glue_t,
    netif: EspNetif,
    driver: EthDriver<'d>,
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> EspEth<'d> {
    pub fn wrap(driver: EthDriver<'d>) -> Result<Self, EspError> {
        Self::wrap_all(driver, EspNetif::new(NetifStack::Eth)?)
    }

    pub fn wrap_all(driver: EthDriver<'d>, netif: EspNetif) -> Result<Self, EspError> {
        let mut this = Self {
            driver,
            netif,
            glue_handle: core::ptr::null_mut(),
        };

        this.attach_netif()?;

        Ok(this)
    }

    pub fn swap_netif(&mut self, netif: EspNetif) -> Result<EspNetif, EspError> {
        self.detach_netif()?;

        let old_netif = core::mem::replace(&mut self.netif, netif);

        self.attach_netif()?;

        Ok(old_netif)
    }

    pub fn driver(&self) -> &EthDriver<'d> {
        &self.driver
    }

    pub fn driver_mut(&mut self) -> &mut EthDriver<'d> {
        &mut self.driver
    }

    pub fn netif(&self) -> &EspNetif {
        &self.netif
    }

    pub fn netif_mut(&mut self) -> &mut EspNetif {
        &mut self.netif
    }

    pub fn start(&mut self) -> Result<(), EspError> {
        self.driver_mut().start()
    }

    pub fn stop(&mut self) -> Result<(), EspError> {
        self.driver_mut().stop()
    }

    pub fn is_started(&self) -> Result<bool, EspError> {
        self.driver().is_started()
    }

    pub fn is_up(&self) -> Result<bool, EspError> {
        Ok(self.driver().is_up()? && self.netif().is_up()?)
    }

    fn attach_netif(&mut self) -> Result<(), EspError> {
        let _ = self.driver.stop();

        let glue_handle = unsafe { esp_eth_new_netif_glue(self.driver.handle()) };

        esp!(unsafe { esp_netif_attach(self.netif.handle(), glue_handle as *mut _) })?;

        self.glue_handle = glue_handle;

        Ok(())
    }

    fn detach_netif(&mut self) -> Result<(), EspError> {
        let _ = self.driver.stop();

        esp!(unsafe { esp_eth_del_netif_glue(self.glue_handle as *mut _) })?;

        self.glue_handle = core::ptr::null_mut();

        Ok(())
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> Drop for EspEth<'d> {
    fn drop(&mut self) {
        self.detach_netif().unwrap();
    }
}

unsafe impl<'d> Send for EspEth<'d> {}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> RawHandle for EspEth<'d> {
    type Handle = *mut esp_eth_netif_glue_t;

    fn handle(&self) -> Self::Handle {
        self.glue_handle
    }
}

impl<'d> Eth for EspEth<'d> {
    type Error = EspError;

    fn start(&mut self) -> Result<(), Self::Error> {
        EspEth::start(self)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        EspEth::stop(self)
    }

    fn is_started(&self) -> Result<bool, Self::Error> {
        EspEth::is_started(self)
    }

    fn is_up(&self) -> Result<bool, Self::Error> {
        EspEth::is_up(self)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EthEvent {
    Started(esp_eth_handle_t),
    Stopped(esp_eth_handle_t),
    Connected(esp_eth_handle_t),
    Disconnected(esp_eth_handle_t),
}

unsafe impl Send for EthEvent {}

impl EthEvent {
    pub fn is_for(&self, raw_handle: impl RawHandle<Handle = esp_eth_handle_t>) -> bool {
        self.is_for_handle(raw_handle.handle())
    }

    pub fn is_for_handle(&self, handle: esp_eth_handle_t) -> bool {
        self.handle() == handle
    }

    pub fn handle(&self) -> esp_eth_handle_t {
        let handle = match self {
            Self::Started(handle) => *handle,
            Self::Stopped(handle) => *handle,
            Self::Connected(handle) => *handle,
            Self::Disconnected(handle) => *handle,
        };

        handle as esp_eth_handle_t
    }
}

impl EspTypedEventSource for EthEvent {
    fn source() -> *const ffi::c_char {
        unsafe { ETH_EVENT }
    }
}

impl EspTypedEventDeserializer<EthEvent> for EthEvent {
    #[allow(non_upper_case_globals, non_snake_case)]
    fn deserialize<R>(
        data: &crate::eventloop::EspEventFetchData,
        f: &mut impl for<'a> FnMut(&'a EthEvent) -> R,
    ) -> R {
        let eth_handle_ref = unsafe { (data.payload as *const esp_eth_handle_t).as_ref() };

        let event_id = data.event_id as u32;

        let event = if event_id == eth_event_t_ETHERNET_EVENT_START {
            EthEvent::Started(*eth_handle_ref.unwrap() as _)
        } else if event_id == eth_event_t_ETHERNET_EVENT_STOP {
            EthEvent::Stopped(*eth_handle_ref.unwrap() as _)
        } else if event_id == eth_event_t_ETHERNET_EVENT_CONNECTED {
            EthEvent::Connected(*eth_handle_ref.unwrap() as _)
        } else if event_id == eth_event_t_ETHERNET_EVENT_DISCONNECTED {
            EthEvent::Disconnected(*eth_handle_ref.unwrap() as _)
        } else {
            panic!("Unknown event ID: {}", event_id);
        };

        f(&event)
    }
}

pub struct EthWait<R> {
    _subscription: EspSubscription<System>,
    waitable: Arc<Waitable<()>>,
    _driver: R,
}

impl<R> EthWait<R> {
    pub fn new(driver: R, sysloop: &EspEventLoop<System>) -> Result<Self, EspError>
    where
        R: RawHandle<Handle = esp_eth_handle_t>,
    {
        let waitable: Arc<Waitable<()>> = Arc::new(Waitable::new(()));

        let s_waitable = waitable.clone();
        let handle = RawHandleImpl(driver.handle());

        let subscription = sysloop
            .subscribe(move |event: &EthEvent| Self::on_eth_event(handle.0, &s_waitable, event))?;

        Ok(Self {
            _driver: driver,
            waitable,
            _subscription: subscription,
        })
    }

    pub fn wait(&self, matcher: impl Fn() -> bool) {
        info!("About to wait");

        self.waitable.wait_while(|_| !matcher());

        info!("Waiting done - success");
    }

    pub fn wait_with_timeout(&self, dur: Duration, matcher: impl Fn() -> bool) -> bool {
        info!("About to wait for duration {:?}", dur);

        let (timeout, _) = self
            .waitable
            .wait_timeout_while_and_get(dur, |_| !matcher(), |_| ());

        if !timeout {
            info!("Waiting done - success");
            true
        } else {
            info!("Timeout while waiting");
            false
        }
    }

    fn on_eth_event(handle: esp_eth_handle_t, waitable: &Waitable<()>, event: &EthEvent) {
        if event.is_for_handle(handle) {
            info!("Got eth event: {:?} ", event);

            waitable.cvar.notify_all();
        }
    }
}
