use crate::stm32::rcc::cfgr::{HPRE_A, SW_A};
use crate::stm32::RCC;

use crate::time::Hertz;

/// Extension trait that constrains the `RCC` peripheral
pub trait RccExt {
    /// Constrains the `RCC` peripheral so it plays nicely with the other abstractions
    fn constrain(self) -> Rcc;
}

impl RccExt for RCC {
    fn constrain(self) -> Rcc {
        Rcc {
            cfgr: CFGR {
                hse: None,
                hclk: None,
                pclk1: None,
                pclk2: None,
                sysclk: None,
                pll48clk: false,
            },
        }
    }
}

/// Constrained RCC peripheral
pub struct Rcc {
    pub cfgr: CFGR,
}

/// Built-in high speed clock frequency
pub const HSI: u32 = 16_000_000; // Hz

/// Minimum system clock frequency
pub const SYSCLK_MIN: u32 = 24_000_000;

/// Maximum system clock frequency
pub const SYSCLK_MAX: u32 = 120_000_000;

/// Maximum APB2 peripheral clock frequency
pub const PCLK2_MAX: u32 = SYSCLK_MAX / 2;
/// Maximum APB1 peripheral clock frequency
pub const PCLK1_MAX: u32 = PCLK2_MAX / 4;

pub struct CFGR {
    hse: Option<u32>,
    hclk: Option<u32>,
    pclk1: Option<u32>,
    pclk2: Option<u32>,
    sysclk: Option<u32>,
    pll48clk: bool,
}

impl CFGR {
    /// Uses HSE (external oscillator) instead of HSI (internal RC oscillator) as the clock source.
    /// Will result in a hang if an external oscillator is not connected or it fails to start.
    pub fn use_hse<F>(mut self, freq: F) -> Self
    where
        F: Into<Hertz>,
    {
        self.hse = Some(freq.into().0);
        self
    }

    pub fn hclk<F>(mut self, freq: F) -> Self
    where
        F: Into<Hertz>,
    {
        self.hclk = Some(freq.into().0);
        self
    }

    pub fn pclk1<F>(mut self, freq: F) -> Self
    where
        F: Into<Hertz>,
    {
        self.pclk1 = Some(freq.into().0);
        self
    }

    pub fn pclk2<F>(mut self, freq: F) -> Self
    where
        F: Into<Hertz>,
    {
        self.pclk2 = Some(freq.into().0);
        self
    }

    pub fn sysclk<F>(mut self, freq: F) -> Self
    where
        F: Into<Hertz>,
    {
        self.sysclk = Some(freq.into().0);
        self
    }

    pub fn require_pll48clk(mut self) -> Self {
        self.pll48clk = true;
        self
    }

    #[inline(always)]
    fn pll_setup(&self) -> (bool, bool, u32, Option<Hertz>) {
        let pllsrcclk = self.hse.unwrap_or(HSI);
        let sysclk = self.sysclk.unwrap_or(pllsrcclk);
        let sysclk_on_pll = sysclk != pllsrcclk;
        if !sysclk_on_pll && !self.pll48clk {
            return (false, false, sysclk, None);
        }

        // Sysclk output divisor must be one of 2, 4, 6 or 8
        let sysclk_div = core::cmp::min(8, (120_000_000 / sysclk) & !1);

        // Input divisor from PLL source clock, must result to frequency in
        // the range from 1 to 2 MHz
        let pllm_min = (pllsrcclk + 1_999_999) / 2_000_000;
        let pllm_max = pllsrcclk / 1_000_000;

        /*
        let target_freq = if self.pll48clk {
            48_000_000
        } else {
            sysclk * sysclk_div
        };
        */
        let target_freq = sysclk * sysclk_div;

        // Find the lowest pllm value that minimize the difference between
        // target frequency and the real vco_out frequency.
        /*
        let pllm = (pllm_min..=pllm_max)
            .min_by_key(|pllm| {
                let vco_in = pllsrcclk / pllm;
                let plln = target_freq / vco_in;
                target_freq - vco_in * plln
            })
            .unwrap();
        */

        let pllm = 20;
        let vco_in = pllsrcclk / pllm;
        //     assert!(vco_in >= 1_000_000 && vco_in <= 2_000_000);

        /*

                // Main scaler, must result in >= 100MHz (>= 192MHz for F401)
                // and <= 120MHz, min 50, max 120
                let _plln = if self.pll48clk {
                    // try the different valid pllq according to the valid
                    // main scaller values, and take the best
                    let pllq = (4..=9)
                        .min_by_key(|pllq| {
                            let plln = 48_000_000 * pllq / vco_in;
                            let pll48_diff = 48_000_000 - vco_in * plln / pllq;
                            let sysclk_diff = (sysclk as i32 - (vco_in * plln / sysclk_div) as i32).abs();
                            (pll48_diff, sysclk_diff)
                        })
                        .unwrap();
                    48_000_000 * pllq / vco_in
                } else {
                    sysclk * sysclk_div / vco_in
                };
        */
        let plln = 192;
        let pllp = 2; // (sysclk_div / 2) - 1;

        //        let pllq = (vco_in * plln + 47_999_999) / 48_000_000;
        let pllq = 5;
        let pll48clk = 1; // vco_in * plln / pllq;

        unsafe { &*RCC::ptr() }.pllcfgr.write(|w| unsafe {
            w.pllm().bits(pllm as u8);
            w.plln().bits(plln as u16);
            w.pllp().bits(pllp as u8);
            w.pllq().bits(pllq as u8);
            w.pllsrc().bit(self.hse.is_some())
        });

        let real_sysclk = if sysclk_on_pll {
            vco_in * plln / sysclk_div
        } else {
            sysclk
        };
        (true, sysclk_on_pll, real_sysclk, None) //Some(Hertz(pll48clk)))
    }

    fn flash_setup(sysclk: u32) {
        use crate::stm32::FLASH;

        let flash_latency_step = 30_000_000;

        unsafe {
            let flash = &(*FLASH::ptr());
            // Adjust flash wait states
            flash.acr.modify(|_, w| {
                w.latency().bits(((sysclk - 1) / flash_latency_step) as u8);
                w.prften().set_bit();
                w.icen().set_bit();
                w.dcen().set_bit()
            })
        }
    }

    pub fn freeze_hack(self) -> Clocks {
        let rcc = unsafe { &*RCC::ptr() };
        let hclk = 120_000_000;
        let sysclk = hclk;
        unsafe { &*RCC::ptr() }.pllcfgr.write(|w| unsafe {
            w.pllm().bits(20 as u8);
            w.plln().bits(192 as u16);
            w.pllp().bits(2 as u8);
            w.pllq().bits(5 as u8);
            w.pllsrc().bit(self.hse.is_some())
        });

        Self::flash_setup(sysclk);
        if self.hse.is_some() {
            rcc.cr.modify(|_, w| w.hseon().set_bit());
            while rcc.cr.read().hserdy().bit_is_clear() {}
        }
        let ppre1 = 4;
        let ppre2 = 4;
        let pclk1 = self
            .pclk1
            .unwrap_or_else(|| core::cmp::min(PCLK1_MAX, hclk));
        let pclk2 = self
            .pclk2
            .unwrap_or_else(|| core::cmp::min(PCLK2_MAX, hclk));
        let sysclk_on_pll = true;
        let use_pll = true;
        if use_pll {
            // Enable PLL
            rcc.cr.modify(|_, w| w.pllon().set_bit());

            // Wait for PLL to stabilise
            while rcc.cr.read().pllrdy().bit_is_clear() {}
        }

        rcc.cfgr.modify(|_, w| unsafe {
            w.ppre2()
                .bits(4)
                .ppre1()
                .bits(4)
                .hpre()
                .variant(HPRE_A::DIV1)
        });

        cortex_m::asm::delay(16);
        rcc.cfgr.modify(|_, w| {
            w.sw().variant(if sysclk_on_pll {
                SW_A::PLL
            } else if self.hse.is_some() {
                SW_A::HSE
            } else {
                SW_A::HSI
            })
        });

        Clocks {
            hclk: Hertz(hclk),
            pclk1: Hertz(pclk1),
            pclk2: Hertz(pclk2),
            ppre1,
            ppre2,
            sysclk: Hertz(sysclk),
            pll48clk: None,
        }
    }

    pub fn freeze(self) -> Clocks {
        let rcc = unsafe { &*RCC::ptr() };

        let (use_pll, sysclk_on_pll, sysclk, pll48clk) = self.pll_setup();

        assert!(!sysclk_on_pll || sysclk <= SYSCLK_MAX && sysclk >= SYSCLK_MIN);

        let hclk = self.hclk.unwrap_or(sysclk);
        let (hpre_bits, hpre_div) = match (sysclk + hclk - 1) / hclk {
            0 => unreachable!(),
            1 => (HPRE_A::DIV1, 1),
            2 => (HPRE_A::DIV2, 2),
            3..=5 => (HPRE_A::DIV4, 4),
            6..=11 => (HPRE_A::DIV8, 8),
            12..=39 => (HPRE_A::DIV16, 16),
            40..=95 => (HPRE_A::DIV64, 64),
            96..=191 => (HPRE_A::DIV128, 128),
            192..=383 => (HPRE_A::DIV256, 256),
            _ => (HPRE_A::DIV512, 512),
        };

        // Calculate real AHB clock
        let hclk = sysclk / hpre_div;

        let pclk1 = self
            .pclk1
            .unwrap_or_else(|| core::cmp::min(PCLK1_MAX, hclk));
        let (ppre1_bits, ppre1) = match (hclk + pclk1 - 1) / pclk1 {
            0 => unreachable!(),
            1 => (0b000, 1),
            2 => (0b100, 2),
            3..=5 => (0b101, 4),
            6..=11 => (0b110, 8),
            _ => (0b111, 16),
        };

        // Calculate real APB1 clock
        let pclk1 = hclk / u32::from(ppre1);

        assert!(pclk1 <= PCLK1_MAX);

        let pclk2 = self
            .pclk2
            .unwrap_or_else(|| core::cmp::min(PCLK2_MAX, hclk));
        let (ppre2_bits, ppre2) = match (hclk + pclk2 - 1) / pclk2 {
            0 => unreachable!(),
            1 => (0b000, 1),
            2 => (0b100, 2),
            3..=5 => (0b101, 4),
            6..=11 => (0b110, 8),
            _ => (0b111, 16),
        };

        // Calculate real APB2 clock
        let pclk2 = hclk / u32::from(ppre2);

        assert!(pclk2 <= PCLK2_MAX);

        Self::flash_setup(sysclk);

        if self.hse.is_some() {
            // enable HSE and wait for it to be ready
            rcc.cr.modify(|_, w| w.hseon().set_bit());
            while rcc.cr.read().hserdy().bit_is_clear() {}
        }

        if use_pll {
            // Enable PLL
            rcc.cr.modify(|_, w| w.pllon().set_bit());

            // Wait for PLL to stabilise
            while rcc.cr.read().pllrdy().bit_is_clear() {}
        }

        // Set scaling factors
        rcc.cfgr.modify(|_, w| unsafe {
            w.ppre2()
                .bits(ppre2_bits)
                .ppre1()
                .bits(ppre1_bits)
                .hpre()
                .variant(hpre_bits)
        });

        // Wait for the new prescalers to kick in
        // "The clocks are divided with the new prescaler factor from 1 to 16 AHB cycles after write"
        cortex_m::asm::delay(16);

        // Select system clock source
        rcc.cfgr.modify(|_, w| {
            w.sw().variant(if sysclk_on_pll {
                SW_A::PLL
            } else if self.hse.is_some() {
                SW_A::HSE
            } else {
                SW_A::HSI
            })
        });

        let clocks = Clocks {
            hclk: Hertz(hclk),
            pclk1: Hertz(pclk1),
            pclk2: Hertz(pclk2),
            ppre1,
            ppre2,
            sysclk: Hertz(sysclk),
            pll48clk,
        };

        if self.pll48clk {
            assert!(clocks.is_pll48clk_valid());
        }

        clocks
    }
}

/// Frozen clock frequencies
///
/// The existence of this value indicates that the clock configuration can no longer be changed
#[derive(Clone, Copy)]
pub struct Clocks {
    hclk: Hertz,
    pclk1: Hertz,
    pclk2: Hertz,
    ppre1: u8,
    ppre2: u8,
    sysclk: Hertz,
    pll48clk: Option<Hertz>,
}

impl Clocks {
    /// Returns the frequency of the AHB1
    pub fn hclk(&self) -> Hertz {
        self.hclk
    }

    /// Returns the frequency of the APB1
    pub fn pclk1(&self) -> Hertz {
        self.pclk1
    }

    /// Returns the frequency of the APB2
    pub fn pclk2(&self) -> Hertz {
        self.pclk2
    }

    /// Returns the prescaler of the APB1
    pub fn ppre1(&self) -> u8 {
        self.ppre1
    }

    /// Returns the prescaler of the APB2
    pub fn ppre2(&self) -> u8 {
        self.ppre2
    }

    /// Returns the system (core) frequency
    pub fn sysclk(&self) -> Hertz {
        self.sysclk
    }

    /// Returns the frequency of the PLL48 clock line
    pub fn pll48clk(&self) -> Option<Hertz> {
        self.pll48clk
    }

    /// Returns true if the PLL48 clock is within USB
    /// specifications. It is required to use the USB functionality.
    pub fn is_pll48clk_valid(&self) -> bool {
        // USB specification allows +-0.25%
        self.pll48clk
            .map(|freq| (48_000_000 - freq.0 as i32).abs() <= 120_000)
            .unwrap_or(false)
    }
}
