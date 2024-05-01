use core::{marker::PhantomData, ops::Deref};

use esp_idf_hal::{gpio::InputPin, peripheral::Peripheral};

#[cfg(esp_idf_soc_sdmmc_use_gpio_matrix)]
use esp_idf_hal::gpio::OutputPin;

#[cfg(not(esp_idf_soc_sdmmc_use_gpio_matrix))]
use esp_idf_hal::gpio;

use crate::sys::*;

pub struct SlotConfiguration<'d> {
    slot: Slot,
    configuration: sdmmc_slot_config_t,
    _p: PhantomData<&'d mut ()>,
}

/// Indicates that card detect line is not used
const SDMMC_SLOT_NO_CD: i32 = -1;
/// Indicates that write protect line is not used
const SDMMC_SLOT_NO_WP: i32 = -1;

#[derive(Clone, Copy, Debug)]
pub enum Slot {
    #[cfg(esp_idf_soc_sdmmc_use_gpio_matrix)]
    NotUsed = 0,
    #[cfg(not(esp_idf_soc_sdmmc_use_gpio_matrix))]
    _0 = 0,
    #[cfg(not(esp_idf_soc_sdmmc_use_gpio_matrix))]
    _1 = 1,
}

impl<'d> SlotConfiguration<'d> {
    #[cfg(esp_idf_soc_sdmmc_use_gpio_matrix)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cmd: impl Peripheral<P = impl OutputPin> + 'd,
        clk: impl Peripheral<P = impl OutputPin> + 'd,
        d0: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        d1: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        d2: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        d3: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        d4: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        d5: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        d6: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        d7: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
    ) -> Self {
        Self {
            slot: Slot::NotUsed,
            configuration: sdmmc_slot_config_t {
                width: Self::get_slot_width(
                    d1.as_ref(),
                    d2.as_ref(),
                    d3.as_ref(),
                    d4.as_ref(),
                    d5.as_ref(),
                    d6.as_ref(),
                    d7.as_ref(),
                ),
                flags: 0,
                __bindgen_anon_1: sdmmc_slot_config_t__bindgen_ty_1 {
                    cd: SDMMC_SLOT_NO_CD,
                },
                __bindgen_anon_2: sdmmc_slot_config_t__bindgen_ty_2 {
                    wp: SDMMC_SLOT_NO_WP,
                },
                clk: clk.into_ref().deref().pin(),
                cmd: cmd.into_ref().deref().pin(),
                d0: d0.into_ref().deref().pin(),
                d1: d1.map(|d1| d1.into_ref().deref().pin()).unwrap_or(-1),
                d2: d2.map(|d2| d2.into_ref().deref().pin()).unwrap_or(-1),
                d3: d3.map(|d3| d3.into_ref().deref().pin()).unwrap_or(-1),
                d4: d4.map(|d4| d4.into_ref().deref().pin()).unwrap_or(-1),
                d5: d5.map(|d5| d5.into_ref().deref().pin()).unwrap_or(-1),
                d6: d6.map(|d6| d6.into_ref().deref().pin()).unwrap_or(-1),
                d7: d7.map(|d7| d7.into_ref().deref().pin()).unwrap_or(-1),
            },
            _p: PhantomData,
        }
    }

    #[cfg(not(esp_idf_soc_sdmmc_use_gpio_matrix))]
    #[allow(clippy::too_many_arguments)]
    pub fn new_slot_0(
        _cmd: impl Peripheral<P = gpio::Gpio11> + 'd,
        _clk: impl Peripheral<P = gpio::Gpio6> + 'd,
        _d0: impl Peripheral<P = gpio::Gpio7> + 'd,
        _d1: Option<impl Peripheral<P = gpio::Gpio8> + 'd>,
        _d2: Option<impl Peripheral<P = gpio::Gpio9> + 'd>,
        _d3: Option<impl Peripheral<P = gpio::Gpio10> + 'd>,
        _d4: Option<impl Peripheral<P = gpio::Gpio16> + 'd>,
        _d5: Option<impl Peripheral<P = gpio::Gpio17> + 'd>,
        _d6: Option<impl Peripheral<P = gpio::Gpio15> + 'd>,
        _d7: Option<impl Peripheral<P = gpio::Gpio18> + 'd>,
    ) -> Self {
        Self {
            slot: Slot::_0,
            configuration: sdmmc_slot_config_t {
                width: Self::get_slot_width(_d1, _d2, _d3, _d4, _d5, _d6, _d7),
                flags: 0,
                __bindgen_anon_1: sdmmc_slot_config_t__bindgen_ty_1 {
                    cd: SDMMC_SLOT_NO_CD,
                },
                __bindgen_anon_2: sdmmc_slot_config_t__bindgen_ty_2 {
                    wp: SDMMC_SLOT_NO_WP,
                },
            },
            _p: PhantomData,
        }
    }

    #[cfg(not(esp_idf_soc_sdmmc_use_gpio_matrix))]
    pub fn new_slot_1(
        _cmd: impl Peripheral<P = gpio::Gpio15> + 'd,
        _clk: impl Peripheral<P = gpio::Gpio14> + 'd,
        _d0: impl Peripheral<P = gpio::Gpio2> + 'd,
        _d1: Option<impl Peripheral<P = gpio::Gpio4> + 'd>,
        _d2: Option<impl Peripheral<P = gpio::Gpio12> + 'd>,
        _d3: Option<impl Peripheral<P = gpio::Gpio13> + 'd>,
    ) -> Self {
        Self {
            slot: Slot::_1,
            configuration: sdmmc_slot_config_t {
                width: Self::get_slot_width(
                    _d1,
                    _d2,
                    _d3,
                    Option::<()>::None,
                    Option::<()>::None,
                    Option::<()>::None,
                    Option::<()>::None,
                ),
                flags: 0,
                __bindgen_anon_1: sdmmc_slot_config_t__bindgen_ty_1 {
                    gpio_cd: SDMMC_SLOT_NO_CD,
                },
                __bindgen_anon_2: sdmmc_slot_config_t__bindgen_ty_2 {
                    gpio_wp: SDMMC_SLOT_NO_WP,
                },
            },
            _p: PhantomData,
        }
    }

    fn get_slot_width<A, B, C, D, E, F, G>(
        _d1: Option<A>,
        _d2: Option<B>,
        _d3: Option<C>,
        _d4: Option<D>,
        _d5: Option<E>,
        _d6: Option<F>,
        _d7: Option<G>,
    ) -> u8 {
        if _d3.is_some() {
            if !(_d2.is_some() && _d1.is_some()) {
                panic!("D2 and D1 must be provided if D3 is provided");
            }

            if _d7.is_some() {
                if !(_d6.is_some() && _d5.is_some() && _d4.is_some()) {
                    panic!("D6, D5, and D4 must be provided if D7 is provided");
                }

                8
            } else {
                4
            }
        } else {
            1
        }
    }

    pub fn set_card_detect_pin(mut self, pin: impl Peripheral<P = impl InputPin> + 'd) -> Self {
        self.configuration.__bindgen_anon_1.gpio_cd = pin.into_ref().deref().pin();
        self
    }

    pub fn set_write_protect_pin(mut self, pin: impl Peripheral<P = impl InputPin> + 'd) -> Self {
        self.configuration.__bindgen_anon_2.gpio_wp = pin.into_ref().deref().pin();
        self
    }

    pub fn get_inner(&self) -> &sdmmc_slot_config_t {
        &self.configuration
    }

    pub fn get_slot(&self) -> Slot {
        self.slot
    }
}
