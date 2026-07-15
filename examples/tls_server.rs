//! Non-blocking TLS echo server using `EspTls::negotiate_server_init` /
//! `negotiate_server_continue`.
//!
//! A single thread drives several handshakes via `select()`: each
//! `negotiate_server_continue` reports `WantRead` / `WantWrite`, and the loop
//! watches the matching fd set. After the handshake, bytes are echoed back.
//!
//! Set `WIFI_SSID` / `WIFI_PASS`, flash, then:
//!
//! ```text
//! openssl s_client -connect <esp-ip>:8443 -quiet
//! ```

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(not(any(esp32h2, esp32h4, esp32p4)), esp_idf_esp_tls_using_mbedtls))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(any(esp32h2, esp32h4, esp32p4))]
fn main() -> anyhow::Result<()> {
    panic!("ESP32-H2, ESP32-H4 and ESP32-P4 do not have a Wifi radio (but you could enable the esp-wifi-remote component to use them with a WiFi co-processor)");
}

#[cfg(all(
    not(any(esp32h2, esp32h4, esp32p4)),
    not(esp_idf_esp_tls_using_mbedtls)
))]
fn main() -> anyhow::Result<()> {
    panic!("This example requires the mbedTLS-based ESP-TLS stack (CONFIG_ESP_TLS_USING_MBEDTLS=y); the server role is always available there.");
}

#[cfg(all(not(any(esp32h2, esp32h4, esp32p4)), esp_idf_esp_tls_using_mbedtls))]
mod example {
    use std::io;
    use std::mem;
    use std::net::{TcpListener, TcpStream};
    use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
    use std::time::{Duration, Instant};

    use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};

    use esp_idf_svc::eventloop::EspSystemEventLoop;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;
    use esp_idf_svc::sys::{self, select, timeval, EspError};
    use esp_idf_svc::tls::{EspTls, ServerConfig, ServerHandshakeStatus, Socket, X509};
    use esp_idf_svc::wifi::{BlockingWifi, EspWifi};

    use log::{error, info, warn};

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    const PORT: u16 = 8443;
    const MAX_CONNECTIONS: usize = 4;

    /// Drop peers that never finish the handshake. Enforced here (via `Instant`);
    /// `tls_handshake_timeout_ms` only applies to the blocking negotiate path.
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

    // Demo self-signed cert/key. Generate your own with:
    //   openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem \
    //       -days 3650 -nodes -subj "/CN=esp32.local"
    // Trailing "\0": ESP-TLS expects NUL-terminated PEM.
    const SERVER_CERT: &[u8] = concat!(include_str!("tls_server_cert.pem"), "\0").as_bytes();
    const SERVER_KEY: &[u8] = concat!(include_str!("tls_server_key.pem"), "\0").as_bytes();

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        // Keep wifi around or it will stop.
        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
            sys_loop,
        )?;
        connect_wifi(&mut wifi)?;

        let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
        info!("Wifi DHCP info: {ip_info:?}");

        let listener = TcpListener::bind(("0.0.0.0", PORT))?;
        listener.set_nonblocking(true)?;
        info!("TLS echo server on port {PORT}; try `openssl s_client -connect <ip>:{PORT} -quiet`");

        let mut conns: Vec<Conn> = Vec::new();

        loop {
            accept_new(&listener, &mut conns);

            let (mut read_fds, mut write_fds, max_fd) = build_fd_sets(&listener, &conns);

            // Poll at least once a second so stalled handshakes hit HANDSHAKE_TIMEOUT.
            let mut tv = timeval {
                tv_sec: 1,
                tv_usec: 0,
            };
            let ret = unsafe {
                select(
                    max_fd + 1,
                    &mut read_fds,
                    &mut write_fds,
                    core::ptr::null_mut(),
                    &mut tv,
                )
            };
            if ret < 0 {
                error!("select() failed: {}", io::Error::last_os_error());
                continue;
            }

            progress(&mut conns, &read_fds, &write_fds);
        }
    }

    fn accept_new(listener: &TcpListener, conns: &mut Vec<Conn>) {
        while conns.len() < MAX_CONNECTIONS {
            let (stream, peer) = match listener.accept() {
                Ok(pair) => pair,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    error!("accept() failed: {e}");
                    break;
                }
            };
            info!("Accepted {peer}");

            // Non-blocking so a stalled peer cannot freeze the accept/select loop.
            // On esp-idf, std's set_nonblocking is ioctl(FIONBIO); lwIP maps that
            // to the same flag as fcntl(O_NONBLOCK), so no extra fcntl is needed.
            if let Err(e) = stream.set_nonblocking(true) {
                warn!("failed to set non-blocking, dropping {peer}: {e}");
                continue;
            }

            let fd = stream.as_raw_fd();
            let mut tls = match EspTls::adopt(TlsSocket::new(stream)) {
                Ok(tls) => tls,
                Err(e) => {
                    error!("adopt failed for {peer}: {e:?}");
                    continue;
                }
            };

            let cfg = ServerConfig {
                server_cert: Some(X509::pem_until_nul(SERVER_CERT)),
                server_key: Some(X509::pem_until_nul(SERVER_KEY)),
                ..ServerConfig::new()
            };

            // Sets the session up only; does not block on the peer.
            if let Err(e) = tls.negotiate_server_init(&cfg) {
                error!("handshake init failed for {peer}: {e:?}");
                continue;
            }

            conns.push(Conn {
                tls,
                fd,
                phase: Phase::Handshaking {
                    deadline: Instant::now() + HANDSHAKE_TIMEOUT,
                    // ClientHello arrives first → watch readability.
                    want_write: false,
                },
            });
        }
    }

    fn progress(conns: &mut Vec<Conn>, read_fds: &sys::fd_set, write_fds: &sys::fd_set) {
        let now = Instant::now();
        let mut i = 0;
        while i < conns.len() {
            match conns[i].phase {
                Phase::Handshaking {
                    deadline,
                    want_write,
                } => {
                    if now >= deadline {
                        warn!("handshake timed out for fd={}", conns[i].fd);
                        conns.swap_remove(i);
                        continue;
                    }

                    let ready = if want_write {
                        fd_isset(conns[i].fd, write_fds)
                    } else {
                        fd_isset(conns[i].fd, read_fds)
                    };
                    if !ready {
                        i += 1;
                        continue;
                    }

                    match conns[i].tls.negotiate_server_continue() {
                        Ok(ServerHandshakeStatus::Complete) => {
                            info!("handshake complete for fd={}", conns[i].fd);
                            conns[i].phase = Phase::Established;
                            i += 1;
                        }
                        // Feed interest back so the next select() watches the right set.
                        Ok(ServerHandshakeStatus::WantRead) => {
                            conns[i].phase = Phase::Handshaking {
                                deadline,
                                want_write: false,
                            };
                            i += 1;
                        }
                        Ok(ServerHandshakeStatus::WantWrite) => {
                            conns[i].phase = Phase::Handshaking {
                                deadline,
                                want_write: true,
                            };
                            i += 1;
                        }
                        Err(e) => {
                            error!("handshake failed for fd={}: {e:?}", conns[i].fd);
                            conns.swap_remove(i);
                        }
                    }
                }
                Phase::Established => {
                    if !fd_isset(conns[i].fd, read_fds) {
                        i += 1;
                        continue;
                    }

                    let mut buf = [0u8; 256];
                    match conns[i].tls.read(&mut buf) {
                        Ok(0) => {
                            info!("peer closed fd={}", conns[i].fd);
                            conns.swap_remove(i);
                        }
                        Ok(n) => {
                            if let Err(e) = conns[i].tls.write_all(&buf[..n]) {
                                warn!("echo write failed for fd={}: {e:?}", conns[i].fd);
                                conns.swap_remove(i);
                            } else {
                                i += 1;
                            }
                        }
                        Err(e) if is_would_block(&e) => i += 1,
                        Err(e) => {
                            warn!("read failed for fd={}: {e:?}", conns[i].fd);
                            conns.swap_remove(i);
                        }
                    }
                }
            }
        }
    }

    struct Conn {
        tls: EspTls<TlsSocket>,
        fd: RawFd,
        phase: Phase,
    }

    #[derive(Clone, Copy)]
    enum Phase {
        Handshaking { deadline: Instant, want_write: bool },
        Established,
    }

    /// Wraps a `TcpStream` for `EspTls::adopt`. On `release()` the fd is handed
    /// to ESP-IDF (via `into_raw_fd`) so TLS drop can close it without a
    /// double-close from `TcpStream`'s own `Drop`.
    struct TlsSocket(Option<TcpStream>);

    impl TlsSocket {
        fn new(stream: TcpStream) -> Self {
            Self(Some(stream))
        }
    }

    impl Socket for TlsSocket {
        fn handle(&self) -> i32 {
            self.0.as_ref().map(|s| s.as_raw_fd()).unwrap_or(-1)
        }

        fn release(&mut self) -> Result<(), EspError> {
            if let Some(stream) = self.0.take() {
                let _ = stream.into_raw_fd();
            }
            Ok(())
        }
    }

    fn build_fd_sets(listener: &TcpListener, conns: &[Conn]) -> (sys::fd_set, sys::fd_set, i32) {
        let mut read_fds: sys::fd_set = unsafe { mem::zeroed() };
        let mut write_fds: sys::fd_set = unsafe { mem::zeroed() };
        let mut max_fd = listener.as_raw_fd();
        fd_set(listener.as_raw_fd(), &mut read_fds);

        for c in conns {
            match c.phase {
                Phase::Handshaking {
                    want_write: true, ..
                } => fd_set(c.fd, &mut write_fds),
                Phase::Handshaking {
                    want_write: false, ..
                }
                | Phase::Established => fd_set(c.fd, &mut read_fds),
            }
            max_fd = max_fd.max(c.fd);
        }

        (read_fds, write_fds, max_fd)
    }

    fn is_would_block(e: &EspError) -> bool {
        let code = e.code();
        code == sys::ESP_TLS_ERR_SSL_WANT_READ || code == sys::ESP_TLS_ERR_SSL_WANT_WRITE
    }

    // Hand-rolled stand-ins for the C `FD_SET` / `FD_ISSET` macros: bindgen does
    // not export those macros, but the layout matches newlib/`sys::fd_set`
    // (`__fds_bits`, bit index = raw VFS fd).
    fn fd_isset(fd: RawFd, set: &sys::fd_set) -> bool {
        let bits = mem::size_of_val(&set.__fds_bits[0]) * 8;
        let fd = fd as usize;
        (set.__fds_bits[fd / bits] & (1 << (fd % bits))) != 0
    }

    fn fd_set(fd: RawFd, set: &mut sys::fd_set) {
        let bits = mem::size_of_val(&set.__fds_bits[0]) * 8;
        let fd = fd as usize;
        set.__fds_bits[fd / bits] |= 1 << (fd % bits);
    }

    fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            password: PASSWORD.try_into().unwrap(),
            auth_method: AuthMethod::WPA2Personal,
            ..Default::default()
        }))?;

        wifi.start()?;
        info!("Wifi started");

        wifi.connect()?;
        info!("Wifi connected");

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        Ok(())
    }
}
