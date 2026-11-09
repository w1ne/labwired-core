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
//! A summary word goes to RAM at 0x2003FF00 (one bit per block passed). TEMP
//! (DATARDY), ECB (AES) and the radio are not exercised: TEMP and ECB are
//! listed as unmodelled residuals in the chip's silicon-conformance block.

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

    wr(RESULT, 0x5284_0000 | summary);
    if summary == 0xFF {
        l.s(b"NRF52840 PASS").send();
    } else {
        l.s(b"NRF52840 FAIL summary=").hex(summary, 2).send();
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
