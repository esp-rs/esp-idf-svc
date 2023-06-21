//! HTTP Server with JSON POST handler

use anyhow;
use embedded_svc::{
    http::{Headers, Method},
    io::{Read, Write},
    wifi::{self, AccessPointConfiguration, AuthMethod},
};
use esp_idf_hal::prelude::Peripherals;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    http::server::EspHttpServer,
    nvs::EspDefaultNvsPartition,
    wifi::{BlockingWifi, EspWifi},
};
use esp_idf_sys::{self as _};
use log::*;
use serde::Deserialize;
use serde_json;

const SSID: &'static str = env!("WIFI_SSID");
const PASSWORD: &'static str = env!("WIFI_PASS");
static INDEX_HTML: &str = include_str!("json_post_handler.html");

// Max payload length
const MAX_LEN: usize = 128;

// Need lots of stack to parse JSON
const STACK_SIZE: usize = 10240;

// Wi-Fi channel, between 1 and 11
const CHANNEL: u8 = 11;

#[derive(Deserialize)]
struct FormData<'a> {
    first_name: &'a str,
    age: u32,
    birthplace: &'a str,
}

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let mut server = create_server();

    server.fn_handler("/", Method::Get, |req| {
        req.into_ok_response()?.write(INDEX_HTML.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler("/post", Method::Post, |mut req| {
        let len = req.content_len().unwrap_or(0) as usize;

        if len > MAX_LEN {
            req.into_status_response(413)?
                .write_all("Request too big".as_bytes())?;
            return Ok(());
        }

        let mut buf = vec![0; len];
        req.read_exact(&mut buf)?;
        let mut resp = req.into_ok_response()?;

        if let Ok(form) = serde_json::from_slice::<FormData>(&buf) {
            write!(
                resp,
                "Hello, {}-year-old {} from {}!",
                form.age, form.first_name, form.birthplace
            )?;
        } else {
            resp.write_all("JSON error".as_bytes())?;
        }

        Ok(())
    })?;

    core::mem::forget(server);

    // Main task no longer needed
    Ok(())
}

fn create_server() -> EspHttpServer {
    let peripherals = Peripherals::take().unwrap();
    let sys_loop = EspSystemEventLoop::take().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs)).unwrap(),
        sys_loop,
    )
    .unwrap();

    let wifi_configuration = wifi::Configuration::AccessPoint(AccessPointConfiguration {
        ssid: SSID.into(),
        ssid_hidden: true,
        auth_method: AuthMethod::WPA2Personal,
        password: PASSWORD.into(),
        channel: CHANNEL,
        ..Default::default()
    });
    wifi.set_configuration(&wifi_configuration).unwrap();
    wifi.start().unwrap();
    wifi.wait_netif_up().unwrap();

    info!("Created wifi with SSID `{}` and PASS `{}`", SSID, PASSWORD);

    let server_configuration = esp_idf_svc::http::server::Configuration {
        stack_size: STACK_SIZE,
        ..Default::default()
    };

    core::mem::forget(wifi);

    EspHttpServer::new(&server_configuration).unwrap()
}
