// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Nano Every (ATmega4809): stock `analogWrite()` on a TCA0 pin and
//! on a TCB pin.
//!
//! The fixture is a stock ArduinoCore-megaavr build (PlatformIO
//! `atmelmegaavr` 1.10.0, ArduinoCore-megaavr 1.8.8) of
//! `examples/arduino-nano-every-pwm/firmware/src/main.cpp`; rebuild with
//! `examples/arduino-nano-every-pwm/firmware/build.sh`, which copies
//! `.pio/build/nano_every/firmware.elf` to
//! `tests/fixtures/avr/arduino-nano-every-pwm.elf`.
//!
//! The core runs TCA0 single-slope at CLK_PER / 64 with PER = 0xFF and every
//! TCB in PWM8 on that clock, so both waveforms have a period of 64 * 256 =
//! 16384 CPU cycles (976.6 Hz) and `analogWrite(pin, n)` is high for 64 * n
//! cycles. D9 = PB0 = TCA0 WO0 on the PORTMUX PORTB route; D3 = PF5 = TCB1 WO
//! on its alternate pad. The widths are measured on the pads, so a wrong
//! route, a lost buffer update or a wrong compare rule moves them.

use crate::common;
use labwired_core::logic_capture::{LogicEdge, LogicSource};
use labwired_core::machine::AdvanceRequest;
use labwired_core::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};

const FIXTURE: &str = "tests/fixtures/avr/arduino-nano-every-pwm.elf";
const PERIOD: u64 = 64 * 256;

struct Run {
    serial: String,
    /// Edges of D9 (channel 0), D3 (channel 1) and D10 (channel 2).
    edges: Vec<LogicEdge>,
    /// Cycle at which each serial line ended.
    line_cycles: Vec<u64>,
    end: u64,
}

/// Run the sketch to its last line, then two more periods.
fn run_sketch() -> Run {
    let chip_path = common::root("configs/chips/atmega4809.yaml");
    let yaml = format!(
        "name: \"nano-every-pwm\"\nchip: \"{}\"\ncpu_hz: 16000000\n",
        chip_path.display()
    );
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let chip = labwired_config::ChipDescriptor::from_file(&chip_path).unwrap();
    let firmware = common::committed(FIXTURE);
    let blobs = BlobMap::new();
    let mut built = build_machine(BuildRequest {
        chip: &chip,
        system: &manifest,
        firmware: FirmwareSource::Elf(&firmware),
        boot: BootMode::FastBoot,
        blobs: &blobs,
        options: BuildOptions::default(),
    })
    .unwrap();
    // Pads are watched by bus index; a second bus from the same descriptors
    // has the same order.
    let names = labwired_core::bus::SystemBus::from_config(&chip, &manifest).unwrap();
    let pad = |port: &str, pin| {
        let idx = names.find_peripheral_index_by_name(port).unwrap();
        Some(LogicSource::pad(idx, pin))
    };
    let watch = [pad("portb", 0), pad("portf", 5), pad("portb", 1)];
    built.machine.logic_watch(&watch);
    let serial = |built: &labwired_core::system::builder::BuiltMachine| {
        String::from_utf8_lossy(&built.uart.sink.lock().unwrap()).into_owned()
    };
    let mut line_cycles = Vec::new();
    let mut tail = 0;
    for _ in 0..4000 {
        built
            .machine
            .advance(AdvanceRequest::run(Some(500)))
            .unwrap();
        let lines = serial(&built).matches('\n').count();
        while line_cycles.len() < lines {
            line_cycles.push(built.machine.logic_now_cycle());
        }
        if lines == 3 {
            tail += 1;
            if tail > 80 {
                break;
            }
        }
    }
    let batch = built.machine.logic_read_edges(0);
    assert_eq!(batch.dropped, 0);
    Run {
        serial: serial(&built),
        edges: batch.edges,
        line_cycles,
        end: built.machine.logic_now_cycle(),
    }
}

/// `(rise cycle, high cycles)` of each complete pulse of `ch` in `from..to`.
fn pulses(run: &Run, ch: u32, from: u64, to: u64) -> Vec<(u64, u64)> {
    let edges: Vec<_> = run.edges.iter().filter(|e| e.ch == ch).collect();
    edges
        .windows(2)
        .filter(|w| w[0].value && !w[1].value && w[0].cycle >= from && w[1].cycle < to)
        .map(|w| (w[0].cycle, w[1].cycle - w[0].cycle))
        .collect()
}

fn assert_waveform(pulses: &[(u64, u64)], high: u64, what: &str) {
    assert!(pulses.len() >= 10, "{what}: {} pulses", pulses.len());
    for pair in pulses.windows(2) {
        assert_eq!(pair[1].0 - pair[0].0, PERIOD, "{what}: period, {pulses:?}");
    }
    for &(rise, width) in pulses {
        assert_eq!(width, high, "{what}: high time of the pulse at {rise}");
        assert_eq!(rise % 64, 0, "{what}: edges sit on the CLK_PER / 64 grid");
    }
}

#[test]
fn nano_every_analog_write_drives_tca0_and_tcb_waveforms_on_their_pads() {
    let run = run_sketch();
    assert_eq!(
        run.serial, "NANO_EVERY_PWM\r\nPWM_HALF\r\nPWM_D3_OFF\r\n",
        "serial:\n{}",
        run.serial
    );
    let [first, half, off] = run.line_cycles[..] else {
        panic!("line cycles {:?}", run.line_cycles);
    };
    // analogWrite(9, 64), then analogWrite(9, 128). The new duty is buffered
    // and taken at the period boundary, so skip one period around the change.
    assert_waveform(&pulses(&run, 0, first, half - PERIOD), 64 * 64, "D9 at 64");
    assert_waveform(
        &pulses(&run, 0, half + PERIOD, run.end),
        64 * 128,
        "D9 at 128",
    );
    // analogWrite(3, 192) until digitalWrite(3, LOW).
    assert_waveform(&pulses(&run, 1, first, off - PERIOD), 64 * 192, "D3 at 192");
    let after_off: Vec<_> = run
        .edges
        .iter()
        .filter(|e| e.ch == 1 && e.cycle > off)
        .collect();
    assert!(after_off.is_empty(), "D3 after turnOffPWM: {after_off:?}");
    let last_d3 = run.edges.iter().rev().find(|e| e.ch == 1).unwrap();
    assert!(!last_d3.value, "D3 ends low");
    // D10 = PB1 = TCA0 WO1 was never enabled.
    assert!(
        !run.edges.iter().any(|e| e.ch == 2),
        "D10 must stay flat: CMP1EN is clear"
    );
}
