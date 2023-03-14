/// Purposefully leaks memory in order to put the value into a static address that FFI functions can access.
///
/// # Notes
///
/// Do not use this function unless you know what you are doing.
/// Especially in loops, this function will cause memory leaks.
#[macro_export]
#[allow(clippy::module_name_repetitions)]
macro_rules! leaky_box_raw {
    ($val:expr) => {
        Box::into_raw(Box::new($val))
    };
}

// #[macro_export]
// macro_rules! leaky_box_u8 {
//     ($val:expr) => {
//         leaky_box_raw!($val) as *mut u8
//     };
// }

// #[macro_export]
// macro_rules! leaky_box_be_bytes {
//     ($val:expr) => {
//         leaky_box_u8!($val.to_be_bytes())
//     };
// }
