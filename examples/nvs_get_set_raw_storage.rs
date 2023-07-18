//! Updates tags from the NVS (default partition) using the native Rust API.
//!
//! Note that this module exposes two separate set of APIs:
//!  * the get_XXX/set_XXX API (where XXX is u8, str, etc.) - this is only for interop with C code that uses the C ESP IDF NVS API as well.
//!  * the `get_raw`/`set_raw` APIs that take a `&[u8]`. This is the "native" Rust API that implements the `RawStorage` trait from `embedded-svc`
//!     and it should be preferred actually, as you can layer on top of it any serde you want.
//!
//! More info regarding NVS:
//!   https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/storage/nvs_flash.html

use esp_idf_sys::{self as _}; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported

use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::*;

use postcard::{from_bytes, to_vec};
use serde::{Deserialize, Serialize};

use log::info;

#[derive(Serialize, Deserialize, Debug)]
struct StructToBeStored<'a> {
    some_bytes: &'a [u8],
    a_str: &'a str,
    a_number: i16,
}

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

    let tag_raw_u8 = "test_raw_u8";
    {
        let tag_raw_u8_data: &[u8] = &[42];

        match nvs.set_raw(tag_raw_u8, tag_raw_u8_data) {
            Ok(_) => info!("Tag updated"),
            // You can find the meaning of the error codes in the output of the error branch in:
            // https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/error-codes.html
            Err(e) => info!("Tag not updated {:?}", e),
        };
    }

    {
        let tag_raw_u8_data: &mut [u8] = &mut [u8::MAX];

        match nvs.get_raw(tag_raw_u8, tag_raw_u8_data) {
            Ok(v) => match v {
                Some(vv) => info!("{:?} = {:?}", tag_raw_u8, vv),
                None => todo!(),
            },
            Err(e) => info!("Couldn't get tag {} because{:?}", tag_raw_u8, e),
        };
    }

    let tag_raw_str: &str = "test_raw_str";
    {
        let tag_raw_str_data = "Hello from the NVS (I'm raw)!";

        match nvs.set_raw(
            tag_raw_str,
            &to_vec::<&str, 100>(&tag_raw_str_data).unwrap(),
        ) {
            Ok(_) => info!("Tag {} updated", tag_raw_str),
            Err(e) => info!("Tag {} not updated {:?}", tag_raw_str, e),
        };
    }

    {
        let tag_raw_str_data: &mut [u8] = &mut [0; 100];

        match nvs.get_raw(tag_raw_str, tag_raw_str_data) {
            Ok(v) => {
                if let Some(the_str) = v {
                    info!("{:?} = {:?}", tag_raw_str, from_bytes::<&str>(the_str));
                }
            }
            Err(e) => info!("Couldn't get tag {} because{:?}", tag_raw_str, e),
        };
    }

    let tag_raw_struct: &str = "test_raw_struct";
    {
        let tag_raw_struct_data = StructToBeStored {
            some_bytes: &[1, 2, 3, 4],
            a_str: "I'm a str inside a struct!",
            a_number: 42,
        };

        match nvs.set_raw(
            tag_raw_struct,
            &to_vec::<StructToBeStored, 100>(&tag_raw_struct_data).unwrap(),
        ) {
            Ok(_) => info!("Tag {} updated", tag_raw_str),
            Err(e) => info!("Tag {} not updated {:?}", tag_raw_str, e),
        };
    }

    {
        let tag_raw_struct_data: &mut [u8] = &mut [0; 100];

        match nvs.get_raw(tag_raw_struct, tag_raw_struct_data) {
            Ok(v) => {
                if let Some(the_struct) = v {
                    info!(
                        "{:?} = {:?}",
                        tag_raw_str,
                        from_bytes::<StructToBeStored>(the_struct)
                    )
                }
            }
            Err(e) => info!("Couldn't get tag {} because{:?}", tag_raw_str, e),
        };
    }

    Ok(())
}
