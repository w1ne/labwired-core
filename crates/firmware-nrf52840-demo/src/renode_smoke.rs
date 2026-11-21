// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_std]
#![no_main]

//! Renode-onboarding smoke for the imported nRF52840 system. Bare registers;
//! each block reads its own peripheral back and reports over UARTE0 (EasyDMA):
//!   FICR    INFO.PART reads 0x52840
//!   NVMC    READY = 1
//!   CLOCK   HFCLKSTART -> EVENTS_HFCLKSTARTED, HFCLKSTAT; LFCLKSTART (XTAL)
//!           -> EVENTS_LFCLKSTARTED, LFCLKSTAT
//!   GPIO    P0.13 output with the input buffer connected: OUT and IN read back
//!   GPIOTE  CONFIG[0] task mode on P0.14: TASKS_OUT toggles the pad (GPIO IN);
//!           the GPIO OUT latch is not GPIOTE's to change. P0.14 is a GPIO
//!           output driving OUT = 0 with its input buffer connected (PIN_CNF =
//!           1), so IN only follows the toggles if it samples the GPIOTE-driven
//!           pad rather than OUT.
//!   TIMER0  32-bit timer, CC[0] compare event, CAPTURE advances, CLEAR resets
//!   RTC0    COUNTER advances, CC[0] compare event (EVTEN.COMPARE0 on, as
//!           nrfx does; the INTEN-only event path is not exercised)
//!   RNG     two EVENTS_VALRDY samples
//!   PPI     EGU0 TRIGGERED[0] -> EGU1 TASKS_TRIGGER[0] only while CH0 is on
//!   SAADC   12-bit conversion of potentiometer input AIN3 through EasyDMA
//! SysTick-to-PendSV tail chaining is also checked on this Cortex-M4 profile,
//! including exception-frame reuse and preservation of the live handler R0.
//!   TEMP    TASKS_START -> EVENTS_DATARDY; TEMP in the -40..85 degC range
//!   ECB     AES-128 FIPS-197 C.1 known-answer test through EasyDMA
//!   TWIM0   TMP102 test fixture on twi0: CONFIG write-mask readback over a
//!           repeated START, reset TEMP 0x1900, ANACK on an empty address
//!   PWM0    one-step EasyDMA sequence on P0.15, 25% duty sampled at the pad
//!           through GPIO IN; TASKS_STOP returns the pad to the GPIO latch
//!   GPIO P1 at its silicon block base 0x50000300: P1.1 drive readback, P1.2
//!           pull-up, GPIOTE PORT=1 toggles on P1.3; P0 OUT and PIN_CNF[31]
//!           unchanged
//!   PWM1/2  INTEN.SEQEND0 pends NVIC line 33 / 34 (and no other line in
//!           28..35); clearing EVENTS_SEQEND0 drops it
//!   SPIM3   hardware chip select (PSEL.CSN) on P0.20 selects a MAX31855
//!           fixture: two 4-byte reads return its power-on frame, the GPIO
//!           latch on P0.20 stays high, INTEN.END pends NVIC line 47
//! A summary word goes to RAM at 0x2003FF00: 0x5280_0000 | one bit per block
//! passed (17 blocks, all pass = 0x5281FFFF). The radio is not exercised.

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "renode_checks.rs"]
mod renode_checks;
use renode_checks::*;

#[entry]
fn main() -> ! {
    let mut summary = 0u32;
    let mut l = Line::new();
    wr(UART_ENABLE, 8);
    l.s(b"NRF52840 RENODE SMOKE").send();

    summary |= chip_checks(&mut l);
    let tailchain_ok = check_systick_pendsv_tailchain(&mut l);
    summary |= u32::from(tailchain_ok) << 8;
    let saadc_ok = check_saadc_ain3(&mut l);
    summary |= u32::from(saadc_ok) << 9;
    summary |= u32::from(check_temp(&mut l)) << 10;
    summary |= u32::from(check_ecb_fips197(&mut l)) << 11;
    summary |= u32::from(check_twim0_tmp102(&mut l)) << 12;
    let (pwm_ok, pwm_high, pwm_rises) = check_pwm0_duty(&mut l);
    summary |= u32::from(pwm_ok) << 13;
    l.s(b"pwm0 samples high=")
        .hex(pwm_high, 4)
        .s(b"/1f40 rises=")
        .hex(pwm_rises, 4)
        .send();
    summary |= u32::from(check_gpio_p1(&mut l)) << 14;
    summary |= u32::from(check_pwm_irq_lines(&mut l)) << 15;
    summary |= u32::from(check_spim3_hw_csn(&mut l)) << 16;

    wr(RESULT, 0x5280_0000 | summary);
    if summary == 0x1_FFFF {
        l.s(b"NRF52840 PASS").send();
    } else {
        l.s(b"NRF52840 FAIL summary=").hex(summary, 5).send();
    }

    // Heartbeat on P0.13 for the gpio_edges check.
    loop {
        wr(GPIO_OUTSET, 1 << LED0);
        for _ in 0..1000u32 {
            let _ = rd(UART_ENABLE);
        }
        wr(GPIO_OUTCLR, 1 << LED0);
        for _ in 0..1000u32 {
            let _ = rd(UART_ENABLE);
        }
    }
}
