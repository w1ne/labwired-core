// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NVIC enable masking must be modelled on the CONFIG-DRIVEN machine.
//!
//! ## The observable
//!
//! Whether an exception handler runs. Real Thumb firmware on a config-built
//! STM32F407 machine: a self-loop at the reset vector, a self-loop at the IRQ0
//! handler. If the handler ran, PC ends inside the handler; if it did not, PC
//! ends inside main. No snapshot, no register read — the CPU's own PC.
//!
//! ## The invariant this pins
//!
//! Cortex-M machines are assembled in TWO steps, and only the second one
//! installs the NVIC:
//!
//!   1. `SystemBus::from_config` has no match arm for `type: "nvic"`, so the 13
//!      chips that declare one fall through the factory chain to
//!      `StubPeripheral::new(0x00)` and leave `bus.nvic = None`.
//!   2. `configure_cortex_m` then finds that entry (by name `"nvic"` OR base
//!      `0xE000_E100` — all 13 declare both) and REPLACES it with the real
//!      `Nvic`, publishing the shared `NvicState` as `bus.nvic`. It does the
//!      same for the `Scb` at `0xE000_ED00`.
//!
//! Every production entry point runs both steps (cli `run`/`test`/`machine`/
//! `debug_probe`, wasm `new_from_config_arm`, dap, python, world/node builder),
//! so the stub is a construction-time transient that never reaches firmware.
//! This test exists because that is a load-bearing ORDERING guarantee held by
//! convention, not by the type system: a future ARM path that calls
//! `from_config` without `configure_cortex_m` would silently get `nvic = None`,
//! and then
//!
//!   * `pend_nvic` (bus/tick.rs:30) pushes the raw IRQ number straight into the
//!     CPU's exception list, skipping ISER entirely;
//!   * `is_nvic_irq_pending` (bus/accessors.rs:775) returns `true`
//!     unconditionally — "No NVIC — assume pending".
//!
//! i.e. `NVIC_ICER` would stop masking anything. These assertions were checked
//! against exactly that shape (a `from_config`-only bus) and all of them fail
//! there, on all 13 chips: `nvic.is_some()=false`, `ISER0` reads back `0x0`
//! after writing `0x5`, SCB `CPUID` reads `0x0`.
//!
//! Both directions are asserted: an ENABLED IRQ must still fire. A regression
//! that masked everything would pass the negative test alone.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;

/// ARMv7-M core peripheral bases (ARM DDI 0403 B3.4).
const NVIC_ISER0: u64 = 0xE000_E100;
const NVIC_ICER0: u64 = 0xE000_E180;
const NVIC_ISPR0: u64 = 0xE000_E200;
const SCB_VTOR: u64 = 0xE000_ED08;

const VECTORS: u32 = 0x0800_0000;
const MAIN: u32 = 0x0800_0100;
const HANDLER: u32 = 0x0800_0200;
const MSP: u32 = 0x2000_8000;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The shipped `nucleo-f407` machine, built exactly the way the run path builds
/// it: `SystemBus::from_config` then `configure_cortex_m`.
fn machine_nucleo_f407() -> Machine<impl Cpu> {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("system yaml");
    let anchored = system_path.parent().expect("parent").join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&manifest.chip).expect("chip yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build f407 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

/// Seed the vector table + two self-loops and park the CPU in main.
fn seed_firmware(m: &mut Machine<impl Cpu>) {
    m.bus.write_u32(VECTORS as u64, MSP).unwrap();
    m.bus.write_u32(VECTORS as u64 + 4, MAIN | 1).unwrap();
    // Exception 16 (external IRQ 0) lives at vector-table offset 16*4 = 0x40.
    m.bus.write_u32(VECTORS as u64 + 0x40, HANDLER | 1).unwrap();

    m.bus.write_u16(MAIN as u64, 0xE7FE).unwrap(); // b .
    m.bus.write_u16(HANDLER as u64, 0xE7FE).unwrap(); // b .

    // Point the core at the table we just wrote (flash base, not 0).
    m.bus.write_u32(SCB_VTOR, VECTORS).unwrap();

    m.cpu.set_pc(MAIN);
    m.cpu.set_sp(MSP);
}

/// Did the core end up inside the IRQ0 handler?
fn handler_ran(m: &Machine<impl Cpu>) -> bool {
    (m.cpu.get_pc() & !1) == HANDLER
}

/// Every chip whose YAML declares `type: "nvic"`. `from_config` has no arm for
/// that type, so each one is a read-zero stub until `configure_cortex_m` runs.
const CHIPS_DECLARING_NVIC: &[&str] = &[
    "stm32f401",
    "stm32f405",
    "stm32f407",
    "stm32f767",
    "stm32g474re",
    "stm32h563",
    "stm32h735",
    "stm32l073",
    "stm32l476",
    "stm32wb55",
    "stm32wba52",
    "rp2040",
    "rp2350",
];

/// The two chips that also declare `type: "scb"` — same stub fallthrough, same
/// repair by `configure_cortex_m` (which installs the real `Scb` at 0xE000_ED00).
const CHIPS_DECLARING_SCB: &[&str] = &["stm32l073", "stm32l476"];

/// SCB CPUID (ARM DDI 0403 B3.2.3) — nonzero on the real model, zero on a stub.
const SCB_CPUID: u64 = 0xE000_ED00;

fn bus_for_chip(chip_name: &str) -> SystemBus {
    let chip_path = root(&format!("configs/chips/{chip_name}.yaml"));
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip yaml");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "schema_version: \"1.0\"\nname: nvic-sweep\nchip: \"{}\"\n",
        chip_path.display()
    ))
    .expect("minimal manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let _ = configure_cortex_m(&mut bus);
    bus
}

/// Fleet ratchet: on EVERY chip that declares `nvic`, the assembled machine must
/// carry the real model and the shared state — not the `from_config` stub.
#[test]
fn every_chip_declaring_nvic_gets_the_real_model() {
    for chip in CHIPS_DECLARING_NVIC {
        let mut bus = bus_for_chip(chip);
        assert!(
            bus.nvic.is_some(),
            "{chip}: bus.nvic is None — NVIC masking unmodelled (pend_nvic bypasses ISER, \
             is_nvic_irq_pending returns true unconditionally)"
        );
        bus.write_u32(NVIC_ISER0, 0x0000_0005).unwrap();
        let readback = bus.read_u32(NVIC_ISER0).unwrap();
        assert_eq!(
            readback, 0x0000_0005,
            "{chip}: NVIC_ISER0 read back {readback:#010x} after writing 0x5 — read-zero stub"
        );
    }
}

/// Same ratchet for the SCB. A real `Scb` model exists and is installed by the
/// same choke point; CPUID reads nonzero on it and zero on the stub.
#[test]
fn every_chip_declaring_scb_gets_the_real_model() {
    for chip in CHIPS_DECLARING_SCB {
        let bus = bus_for_chip(chip);
        let cpuid = bus.read_u32(SCB_CPUID).unwrap();
        assert_ne!(
            cpuid, 0,
            "{chip}: SCB CPUID reads 0 — that is the from_config stub, not the Scb model"
        );
    }
}

/// Precondition, stated as its own assertion: the config path must install the
/// real NVIC model, not the read-zero stub, and must publish the shared state.
#[test]
fn config_path_installs_a_real_nvic() {
    let mut m = machine_nucleo_f407();

    assert!(
        m.bus.nvic.is_some(),
        "bus.nvic is None on a config-built Cortex-M machine: NVIC masking cannot be modelled \
         (pend_nvic bypasses ISER, is_nvic_irq_pending returns true unconditionally)"
    );

    // A StubPeripheral(0x00) swallows the write and reads back zero.
    m.bus.write_u32(NVIC_ISER0, 0x0000_0005).unwrap();
    let readback = m.bus.read_u32(NVIC_ISER0).unwrap();
    assert_eq!(
        readback, 0x0000_0005,
        "NVIC_ISER0 read back {readback:#010x} after writing 0x5 — that is a read-zero stub, \
         not the Nvic model"
    );

    // ICER must clear the enable bits it is written with.
    m.bus.write_u32(NVIC_ICER0, 0x0000_0004).unwrap();
    let after_icer = m.bus.read_u32(NVIC_ISER0).unwrap();
    assert_eq!(
        after_icer, 0x0000_0001,
        "NVIC_ICER0 write of 0x4 must clear ISER bit 2, leaving 0x1 (got {after_icer:#010x})"
    );
}

/// Positive direction: an ENABLED, pended IRQ still reaches its handler.
#[test]
fn enabled_irq_still_fires() {
    let mut m = machine_nucleo_f407();
    seed_firmware(&mut m);

    m.bus.write_u32(NVIC_ISER0, 1).unwrap(); // enable IRQ0
    m.bus.write_u32(NVIC_ISPR0, 1).unwrap(); // pend IRQ0

    m.run(Some(64)).unwrap();

    assert!(
        handler_ran(&m),
        "an ENABLED, pended IRQ0 must dispatch: PC={:#010x}, expected the handler at {HANDLER:#010x}",
        m.cpu.get_pc()
    );
}

/// Negative direction: firmware disables IRQ0 via `NVIC_ICER`, then the source
/// pends it. The handler must NOT run.
#[test]
fn icer_disabled_irq_does_not_fire() {
    let mut m = machine_nucleo_f407();
    seed_firmware(&mut m);

    m.bus.write_u32(NVIC_ISER0, 1).unwrap(); // enable IRQ0 ...
    m.bus.write_u32(NVIC_ICER0, 1).unwrap(); // ... then mask it off
    m.bus.write_u32(NVIC_ISPR0, 1).unwrap(); // source pends anyway

    m.run(Some(64)).unwrap();

    assert!(
        !handler_ran(&m),
        "IRQ0 was DISABLED via NVIC_ICER but its handler ran anyway (PC={:#010x}). \
         NVIC masking is not modelled on this machine.",
        m.cpu.get_pc()
    );
}
