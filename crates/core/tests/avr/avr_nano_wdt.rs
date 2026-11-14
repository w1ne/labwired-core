// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Nano watchdog: the stock `<avr/wdt.h>` sketch in
//! examples/arduino-nano-wdt runs on the ATmega328P model. It covers the
//! interrupt mode, `wdt_reset()`, the power-down wake, a reset on hang that
//! the next boot sees as `MCUSR.WDRF`, and interrupt-then-reset mode.
//!
//! Before the watchdog was modelled, `WDR` did not decode, so this sketch
//! stopped with a decode error in the `wdt_disable()` of its `.init3`, before
//! `main()` ran.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn nano_watchdog_interrupts_wakes_and_resets() {
    let yaml = root().join("configs/systems/arduino-nano.yaml");
    let manifest = SystemManifest::from_file(&yaml).expect("load system yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-nano-wdt.elf"))
        .expect("missing fixture; build examples/arduino-nano-wdt");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega328p);
    cpu.load_program_image(&image);
    cpu.set_wdt_cpu_hz(16_000_000);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(serial.clone());
    let mut machine = Machine::new(cpu, bus);

    let text = |s: &Arc<Mutex<Vec<u8>>>| String::from_utf8_lossy(&s.lock().unwrap()).into_owned();
    let mut hang_at = None;
    let mut reset_at = None;
    for _ in 0..12_000_000u32 {
        machine.step().expect("step");
        if hang_at.is_none() && text(&serial).contains("hang\r\n") {
            hang_at = Some(machine.cpu.cycles);
        }
        if hang_at.is_some() && reset_at.is_none() && machine.cpu.pc == 0 {
            reset_at = Some(machine.cpu.cycles);
        }
        if text(&serial).contains("WDT DONE") {
            break;
        }
    }
    let out = text(&serial);
    for line in [
        "boot 0 mcusr=00 wdtcsr=0\r\n",
        "isr=6\r\n",
        "kick OK\r\n",
        "pwrdown woke hits=1 dmillis=0\r\n",
        "boot 1 mcusr=08 wdtcsr=0\r\n",
        "boot 2 mcusr=08 wdtcsr=0\r\n",
        "irq_then_reset=1\r\n",
        "WDT DONE",
    ] {
        assert!(out.contains(line), "missing {line:?}; serial: {out:?}");
    }
    // WDTO_120MS is 16K cycles of the 128 kHz oscillator: 128 ms, 2 048 000
    // clocks at 16 MHz from the wdr inside wdt_enable() to the reset. The
    // "hang" newline is loaded into UDR0 just before; Serial.flush() then
    // waits for it to shift out.
    let (hang, reset) = (
        hang_at.unwrap(),
        reset_at.expect("watchdog never reset the core"),
    );
    let gap = reset - hang;
    eprintln!("hang -> reset: {gap} clocks");
    assert!(
        (2_048_000..=2_052_000).contains(&gap),
        "reset {gap} clocks after the hang, want ~2 048 000"
    );
}
