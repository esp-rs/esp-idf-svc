// Loop that updates tags from the NVS.
use esp_idf_svc::nvs::*;

fn main() -> anyhow::Result<()> {
    let nvs_default_partition: EspNvsPartition<NvsDefault> =
        EspDefaultNvsPartition::take().unwrap();

    let test_namespace = "test_ns";
    let mut nvs = match EspNvs::new(nvs_default_partition, test_namespace, true) {
        Ok(nvs) => {
            println!("Got namespace {:?} from default partition", test_namespace);
            nvs
        }
        Err(e) => panic!("Could't get namespace {:?}", e),
    };

    let tag_u8 = "test_u8";

    match nvs.set_u8(tag_u8, 42) {
        Ok(_) => println!("Tag updated"),
        // You can find the meaning of the error codes in the output of the error branch in:
        // https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/error-codes.html
        Err(e) => println!("Tag not updated {:?}", e),
    };

    match nvs.get_u8(tag_u8).unwrap() {
        Some(v) => println!("{:?} = {:?}", tag_u8, v),
        None => println!("{:?} not found", tag_u8),
    };

    let tag_test_str = "test_str";
    // String values are limited in the IDF to 4000 bytes, but our buffer is shorter.
    const MAX_STR_LEN: usize = 100;

    let the_str_len: usize = nvs.str_len(tag_test_str).map_or(0, |v| {
        println!("Got stored string length of {:?}", v);
        let vv = v.unwrap_or(0);
        if vv >= MAX_STR_LEN {
            println!("Too long, trimming");
            0
        } else {
            vv
        }
    });

    match the_str_len == 0 {
        true => println!("{:?} does not seem to exist", tag_test_str),
        false => {
            let mut buffer: [u8; MAX_STR_LEN] = [0; MAX_STR_LEN];
            match nvs.get_str(tag_test_str, &mut buffer).unwrap() {
                Some(v) => println!("{:?} = {:?}", tag_test_str, v),
                None => println!("WE got nothing from {:?}", tag_test_str),
            };
        }
    };

    match nvs.set_str(tag_test_str, "Hello from the NVS!") {
        Ok(_) => println!("{:?} updated", tag_test_str),
        Err(e) => println!("{:?} not updated {:?}", tag_test_str, e),
    };

    // set_str uses CString, that returns an error indicating that an interior nul byte was found.
    // This is going to return an error.
    match nvs.set_str(tag_test_str, "Hello\0from\0the\0NVS\0") {
        Ok(_) => println!("This should not happen!"),
        Err(e) => println!("{:?} not updated ({:?})", tag_test_str, e),
    };

    Ok(())
}
