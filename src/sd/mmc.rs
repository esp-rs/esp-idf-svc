use crate::sys::*;

pub struct SlotConfiguration(sdmmc_slot_config_t);

/// Indicates that card detect line is not used
const SDMMC_SLOT_NO_CD: i32 = -1;
/// Indicates that write protect line is not used
const SDMMC_SLOT_NO_WP: i32 = -1;
/// Use the maximum possible width for the slot
const SDMMC_SLOT_WIDTH_DEFAULT: u8 = 0;

impl Default for SlotConfiguration {
    fn default() -> Self {
        Self(sdmmc_slot_config_t {
            width: SDMMC_SLOT_WIDTH_DEFAULT,
            flags: 0,
            __bindgen_anon_1: sdmmc_slot_config_t__bindgen_ty_1 {
                // ? : Why union here?
                gpio_cd: SDMMC_SLOT_NO_CD,
            },
            __bindgen_anon_2: sdmmc_slot_config_t__bindgen_ty_2 {
                // ? : Why union here?
                gpio_wp: SDMMC_SLOT_NO_WP,
            },
        })
    }
}

impl SlotConfiguration {
    pub fn set_width(mut self, width: u8) -> Self {
        self.0.width = width;
        self
    }

    pub fn set_card_detect_pin(mut self, pin: gpio_num_t) -> Self {
        self.0.__bindgen_anon_1.gpio_cd = pin;
        self
    }

    pub fn set_write_protect_pin(mut self, pin: gpio_num_t) -> Self {
        self.0.__bindgen_anon_2.gpio_wp = pin;
        self
    }

    pub fn get_inner(&self) -> &sdmmc_slot_config_t {
        &self.0
    }
}
