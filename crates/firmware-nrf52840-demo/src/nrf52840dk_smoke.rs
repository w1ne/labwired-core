// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_std]
#![no_main]

//! Renode-onboarding smoke for the nRF52840 DK board row
//! (Renode: boards/nrf52840dk_nrf52840.repl = cpus/nrf52840.repl plus LED1-4
//! on P0.13-16 and Button 1-4 on P0.11/12/24/25). Runs the eight nRF52840
//! silicon blocks of the chip row (`renode_checks.rs`, bits 0..7), then the
//! board IO (PCA10056 user guide §8.7: LEDs and buttons are active low, the
//! buttons short the pin to GND and need the internal pull-up):
//!   LEDS    each LED lit alone through OUTCLR (OUT reads all-but-one high),
//!           then all four off again                              (bit 8)
//!   BUTTONS P0.11/12/24/25 inputs with the pull-up: idle reads high; each
//!           button pressed and released once by the smoke stimuli, in order,
//!           with no other button moving                          (bit 9)
//!   GPIOTE  CONFIG[1] Event mode on P0.11, polarity HiToLo, INTEN IN1: the
//!           next press of Button 1 sets EVENTS_IN[1] and pends GPIOTE
//!           (NVIC line 6, left disabled so the pend stays visible); the
//!           release (LoToHi) does not                            (bit 10)
//! A summary word goes to RAM at 0x2003FF00.

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "renode_checks.rs"]
mod renode_checks;
use renode_checks::*;

const LEDS: [u32; 4] = [13, 14, 15, 16];
const BUTTONS: [u32; 4] = [11, 12, 24, 25];
const LED_MASK: u32 = 0xF << 13;
const BUTTON_MASK: u32 = (1 << 11) | (1 << 12) | (1 << 24) | (1 << 25);

/// PIN_CNF: DIR = input, INPUT = connect, PULL = pull-up (3 << 2).
const PIN_CNF_INPUT_PULLUP: u32 = 3 << 2;

const GPIOTE_EVENTS_IN1: u32 = 0x4000_6104;
const GPIOTE_INTENSET: u32 = 0x4000_6304;
const GPIOTE_INTENCLR: u32 = 0x4000_6308;
const GPIOTE_CONFIG1: u32 = 0x4000_6514;
const GPIOTE_IRQ: u32 = 6;

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;

/// Button waits: the stimuli are a few hundred thousand cycles apart.
const PRESS_SPIN: u32 = 1_500_000;

fn buttons() -> u32 {
    rd(GPIO_IN) & BUTTON_MASK
}

/// Poll until the button pins read `want`; false on timeout.
fn wait_buttons(want: u32) -> bool {
    for _ in 0..PRESS_SPIN {
        if buttons() == want {
            return true;
        }
    }
    false
}

#[entry]
fn main() -> ! {
    let mut summary = 0u32;
    let mut l = Line::new();
    wr(UART_ENABLE, 8);
    l.s(b"NRF52840-DK RENODE SMOKE").send();

    summary |= chip_checks(&mut l);

    // LEDs: outputs, all off (high), then each lit alone (low).
    for pin in LEDS {
        wr(GPIO_PIN_CNF + pin * 4, 1);
    }
    wr(GPIO_OUTSET, LED_MASK);
    let off = rd(GPIO_OUT) & LED_MASK == LED_MASK;
    let mut alone = true;
    for pin in LEDS {
        wr(GPIO_OUTCLR, 1 << pin);
        alone &= rd(GPIO_OUT) & LED_MASK == LED_MASK & !(1 << pin);
        alone &= rd(GPIO_IN) & LED_MASK == LED_MASK & !(1 << pin);
        for _ in 0..2000u32 {
            let _ = rd(UART_ENABLE);
        }
        wr(GPIO_OUTSET, 1 << pin);
    }
    let off_again = rd(GPIO_OUT) & LED_MASK == LED_MASK;
    let pass = off && alone && off_again;
    l.s(b"leds p0.13-16 off=")
        .hex(off as u32, 1)
        .s(b" alone=")
        .hex(alone as u32, 1)
        .s(b" off=")
        .hex(off_again as u32, 1)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 8;

    // Buttons: pulled-up inputs, pressed one at a time by the stimuli.
    for pin in BUTTONS {
        wr(GPIO_PIN_CNF + pin * 4, PIN_CNF_INPUT_PULLUP);
    }
    let idle = buttons() == BUTTON_MASK;
    let mut presses = 0u32;
    for pin in BUTTONS {
        let pressed = wait_buttons(BUTTON_MASK & !(1 << pin));
        let released = pressed && wait_buttons(BUTTON_MASK);
        presses += (pressed && released) as u32;
    }
    let pass = idle && presses == 4;
    l.s(b"buttons idle=")
        .hex(idle as u32, 1)
        .s(b" presses=")
        .hex(presses, 1)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 9;

    // GPIOTE Event mode on Button 1: HiToLo (POLARITY = 2) on P0.11.
    wr(NVIC_ICPR0, 1 << GPIOTE_IRQ);
    wr(GPIOTE_EVENTS_IN1, 0);
    wr(GPIOTE_CONFIG1, 1 | (BUTTONS[0] << 8) | (2 << 16));
    wr(GPIOTE_INTENSET, 1 << 1);
    let quiet = rd(GPIOTE_EVENTS_IN1) == 0 && rd(NVIC_ISPR0) & (1 << GPIOTE_IRQ) == 0;
    let pressed = wait_buttons(BUTTON_MASK & !(1 << BUTTONS[0]));
    settle();
    let fired = rd(GPIOTE_EVENTS_IN1) != 0;
    let pend = rd(NVIC_ISPR0) & (1 << GPIOTE_IRQ) != 0;
    wr(GPIOTE_EVENTS_IN1, 0);
    let released = wait_buttons(BUTTON_MASK);
    settle();
    let no_rise = rd(GPIOTE_EVENTS_IN1) == 0;
    wr(GPIOTE_INTENCLR, 1 << 1);
    wr(GPIOTE_CONFIG1, 0);
    wr(NVIC_ICPR0, 1 << GPIOTE_IRQ);
    let pass = quiet && pressed && fired && pend && released && no_rise;
    l.s(b"gpiote in1 p0.11 quiet=")
        .hex(quiet as u32, 1)
        .s(b" fall=")
        .hex(fired as u32, 1)
        .s(b" pend=")
        .hex(pend as u32, 1)
        .s(b" rise=")
        .hex(!no_rise as u32, 1)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 10;

    wr(RESULT, 0x5284_0000 | summary);
    if summary == 0x7FF {
        l.s(b"NRF52840-DK PASS").send();
    } else {
        l.s(b"NRF52840-DK FAIL summary=").hex(summary, 3).send();
    }

    // Heartbeat on LED1 (P0.13) for the gpio_edges check.
    loop {
        wr(GPIO_OUTCLR, 1 << LED0);
        for _ in 0..1000u32 {
            let _ = rd(UART_ENABLE);
        }
        wr(GPIO_OUTSET, 1 << LED0);
        for _ in 0..1000u32 {
            let _ = rd(UART_ENABLE);
        }
    }
}
