//! Updates tags from the NVS (default partition).
//!
//! Note that this module exposes two separate set of APIs:
//!  * the get_XXX/set_XXX API (where XXX is u8, str, etc.) - this is only for interop with C code that uses the C ESP IDF NVS API as well.
//!  * the `get_raw`/`set_raw` APIs that take a `&[u8]`. This is the "native" Rust API that implements the `RawStorage` trait from `embedded-svc` and it should be preferred actually, as you can layer on top of it any serde you want.
//!
//! More info reagarding NVS:
//!   https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/storage/nvs_flash.html

use esp_idf_sys::{self as _}; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported

use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::*;
use log::info;

fn main() -> anyhow::Result<()> {
    EspLogger::initialize_default();

    let nvs_default_partition: EspNvsPartition<NvsDefault> =
        EspDefaultNvsPartition::take().unwrap();

    let test_namespace = "test_ns";
    let mut nvs = match EspNvs::new(nvs_default_partition, test_namespace, true) {
        Ok(nvs) => {
            info!("Got namespace {:?} from default partition", test_namespace);
            nvs
        }
        Err(e) => panic!("Could't get namespace {:?}", e),
    };

    let tag_u8 = "test_u8";

    match nvs.set_u8(tag_u8, 42) {
        Ok(_) => info!("Tag updated"),
        // You can find the meaning of the error codes in the output of the error branch in:
        // https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/error-codes.html
        Err(e) => info!("Tag not updated {:?}", e),
    };

    match nvs.get_u8(tag_u8).unwrap() {
        Some(v) => info!("{:?} = {:?}", tag_u8, v),
        None => info!("{:?} not found", tag_u8),
    };

    let tag_test_str = "test_str";
    // String values are limited in the IDF to 4000 bytes, but our buffer is shorter.
    const MAX_STR_LEN: usize = 100;

    let the_str_len: usize = nvs.str_len(tag_test_str).map_or(0, |v| {
        info!("Got stored string length of {:?}", v);
        let vv = v.unwrap_or(0);
        if vv >= MAX_STR_LEN {
            info!("Too long, trimming");
            0
        } else {
            vv
        }
    });

    match the_str_len == 0 {
        true => info!("{:?} does not seem to exist", tag_test_str),
        false => {
            let mut buffer: [u8; MAX_STR_LEN] = [0; MAX_STR_LEN];
            match nvs.get_str(tag_test_str, &mut buffer).unwrap() {
                Some(v) => info!("{:?} = {:?}", tag_test_str, v),
                None => info!("We got nothing from {:?}", tag_test_str),
            };
        }
    };

    match nvs.set_str(tag_test_str, "Hello from the NVS!") {
        Ok(_) => info!("{:?} updated", tag_test_str),
        Err(e) => info!("{:?} not updated {:?}", tag_test_str, e),
    };

    Ok(())
}
