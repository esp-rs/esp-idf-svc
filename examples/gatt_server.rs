use std::sync::{Arc, RwLock};

use esp_idf_svc::ble::{
    gatt_server::{Characteristic, Profile, Service, GLOBAL_GATT_SERVER},
    utilities::{AttributePermissions, BleUuid, CharacteristicProperties},
};

use esp_idf_sys::{esp_get_free_heap_size, esp_get_free_internal_heap_size};
use log::info;

fn main() {
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Logger initialised.");

    let char_value_write: Arc<RwLock<Vec<u8>>> =
        Arc::new(RwLock::new("Initial value".as_bytes().to_vec()));
    let char_value_read = char_value_write.clone();

    // A static characteristic.
    let static_characteristic = Characteristic::new(BleUuid::from_uuid128_string(
        "d4e0e0d0-1a2b-11e9-ab14-d663bd873d93",
    ))
    .name("Static Characteristic")
    .permissions(AttributePermissions::new().read())
    .max_value_length(20)
    .properties(CharacteristicProperties::new().read())
    .show_name()
    .set_value("Hello, world!".as_bytes().to_vec())
    .build();

    // A characteristic that notifies every second.
    let notifying_characteristic = Characteristic::new(BleUuid::from_uuid128_string(
        "a3c87500-8ed3-4bdf-8a39-a01bebede295",
    ))
    .name("Notifying Characteristic")
    .permissions(AttributePermissions::new().read())
    .properties(CharacteristicProperties::new().read().notify())
    .max_value_length(20)
    .show_name()
    .set_value("Initial value.".as_bytes().to_vec())
    .build();

    // A characteristic that notifies every second.
    let indicating_characteristic = Characteristic::new(BleUuid::from_uuid128_string(
        "c41d6f80-1a2c-11e9-ab14-d663bd873d93",
    ))
    .name("Indicating Characteristic")
    .permissions(AttributePermissions::new().read())
    .properties(CharacteristicProperties::new().read().indicate())
    .max_value_length(20)
    .show_name()
    .set_value("Initial value.".as_bytes().to_vec())
    .build();

    // A writable characteristic.
    let writable_characteristic = Characteristic::new(BleUuid::from_uuid128_string(
        "3c9a3f00-8ed3-4bdf-8a39-a01bebede295",
    ))
    .name("Writable Characteristic")
    .permissions(AttributePermissions::new().read().write())
    .properties(CharacteristicProperties::new().read().write())
    .on_read(move |_param| {
        info!("Read from writable characteristic.");
        return char_value_read.read().unwrap().clone();
    })
    .on_write(move |value, _param| {
        info!("Wrote to writable characteristic: {:?}", value);
        *char_value_write.write().unwrap() = value;
    })
    .show_name()
    .build();

    let service = Service::new(BleUuid::from_uuid128_string(
        "fafafafa-fafa-fafa-fafa-fafafafafafa", // far better, run run run run, run run run away...
    ))
    .name("Example Service")
    .primary()
    .characteristic(&static_characteristic)
    .characteristic(&notifying_characteristic)
    .characteristic(&indicating_characteristic)
    .characteristic(&writable_characteristic)
    .build();

    let profile = Profile::new(0x0001)
        .name("Default Profile")
        .service(&service)
        .build();

    GLOBAL_GATT_SERVER
        .lock()
        .unwrap()
        .profile(profile)
        .device_name("ESP32-GATT-Server")
        .appearance(esp_idf_svc::ble::utilities::Appearance::WristWornPulseOximeter)
        .advertise_service(&service)
        .start();

    std::thread::spawn(move || {
        let mut counter = 0;
        loop {
            counter += 1;
            notifying_characteristic
                .write()
                .unwrap()
                .set_value(format!("Counter: {counter}").as_bytes().to_vec());
            indicating_characteristic
                .write()
                .unwrap()
                .set_value(format!("Counter: {counter}").as_bytes().to_vec());
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });

    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(500));

        unsafe {
            let x = esp_get_free_heap_size();
            let y = esp_get_free_internal_heap_size();
            info!("Free heap: {} bytes, free internal heap: {} bytes", x, y);
        }
    });

    loop {
        info!("Main loop.");
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}
