// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! An LED on an ATmega328P pad, as the diagram compiler lowers it: the pad is
//! a Thevenin source (the chip's `io_voltage_v` behind its `gpio_output_ohms`,
//! 25 ohm) and the LED is a diode card fitted to its datasheet forward curve
//! (red, Kingbright WP7113ID: 1.9 V at 10 mA, 2.0 V at 20 mA, giving
//! IS = 3.799e-18, N = 2, RS = 6.416). The firmware drives PB1 high and parks.
//!
//! Nothing limits the current but physics: with no resistor the pad's own
//! output resistance and the junction share 5 V, and the pad carries several
//! times the chip's 40 mA absolute maximum; a 220 ohm series resistor holds
//! it to a normal indicator current. The pad's own current is probed as
//! `pad_i_pb1` = i(Vdrv_pb1), negative while the pad sources.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, RoutingError};
use labwired_core::cpu::avr::Avr;
use labwired_core::{AdvanceRequest, Machine};
use std::path::Path;

/// `SBI DDRB,1 ; SBI PORTB,1 ; RJMP .` — PB1 high, then park.
const PB1_HIGH: [u16; 3] = [0x9A21, 0x9A29, 0xCFFF];
const CPU_HZ: u64 = 16_000_000;

/// `SERIES` is the netlist between the pad node and the LED anode: a resistor
/// line, or nothing (the LED's anode on the pad itself).
const LED_SYSTEM: &str = r#"
cosim_models:
  - id: "circuit"
    adapter: "analog"
    step_ns: 100000
    inputs:
      drv_pb1: "board.gpio_pad.pb1"
      ctl_pb1: "board.gpio_driven.pb1"
    outputs:
      in_pb1: "board.gpio_in.pb1"
    config:
      vdd: 5.0
      netlist_text: |
        Vdrv_pb1 d_pb1 0 dc 0
        Sdrv_pb1 d_pb1 pad ctl_pb1 ron=25 roff=1e12
        SERIES
        DD1 anode 0 LED_RED
        .model LED_RED D(IS=3.799e-18 N=2 RS=6.416)
      probes:
        in_pb1: "v(pad)"
        pad_i_pb1: "i(Vdrv_pb1)"
        led_D1: "i(DD1)"
      sources:
        drv_pb1: "Vdrv_pb1"
"#;

/// Settled (pad current, LED current) in amps after 50 ms with PB1 high.
fn settled(series_line: &str) -> (f64, f64) {
    let system = LED_SYSTEM.replace("SERIES", series_line);
    let chip_path = root("configs/chips/atmega328p.yaml");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"led-on-pin\"\nchip: \"{}\"\nexternal_devices: []\n{system}",
        chip_path.display()
    ))
    .expect("the system parses");
    let descriptor = ChipDescriptor::from_file(chip_path).expect("load chip descriptor");
    let bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
    let mut cpu = Avr::new();
    cpu.load_words(0, &PB1_HIGH);
    let mut machine = Machine::new(cpu, bus);
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
        .expect("build session")
        .expect("a model was declared");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    machine.attach_analog_trace(session.analog_trace_registry());
    while machine.total_cycles < 50 * CPU_HZ / 1000 {
        session
            .advance(&mut machine, AdvanceRequest::run(None))
            .expect("advance");
    }
    let series = machine.analog_series().expect("an analog series");
    let last = |name: &str| {
        series
            .channels
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.points.last())
            .map(|p| p.1)
            .unwrap_or_else(|| panic!("channel {name}"))
    };
    (last("circuit.pad_i_pb1"), last("circuit.led_D1"))
}

#[test]
fn an_led_straight_off_a_pin_draws_what_the_pad_resistance_allows() {
    let (pad, led) = settled("Rwire pad anode 1e-3");
    eprintln!(
        "LED on PB1, no resistor: pad {:.1} mA, LED {:.1} mA",
        pad * 1e3,
        led * 1e3
    );
    // The pad sources (negative branch current) exactly what the LED carries.
    assert!(pad < 0.0, "the pad sources: {pad} A");
    assert!((pad.abs() - led).abs() < 1e-4, "pad {pad} A vs LED {led} A");
    // Tens of mA, well past the ATmega328P's 40 mA absolute maximum.
    assert!(led > 0.060 && led < 0.130, "LED current {led} A");
}

#[test]
fn a_220_ohm_resistor_holds_the_led_to_an_indicator_current() {
    let (pad, led) = settled("R1 pad anode 220");
    eprintln!(
        "LED on PB1 through 220 ohm: pad {:.1} mA, LED {:.1} mA",
        pad * 1e3,
        led * 1e3
    );
    assert!(pad < 0.0, "the pad sources: {pad} A");
    assert!(led > 0.010 && led < 0.015, "LED current {led} A");
}
