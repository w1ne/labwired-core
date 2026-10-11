// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Pulls that live outside the GPIO block reach the pad.
//!
//! Kinetis keeps a pad's pull in `PORTx_PCRn`, Renesas RA in `PmnPFS`, i.MX
//! RT in the IOMUXC `SW_PAD_CTL_PAD_*` registers. Firmware writes those, not
//! the GPIO port, so the in-tree chips name the block on the port
//! (`pad_control`) and the bus hands the decoded pull over after each write.
//! Each case below boots the real chip yaml, writes the pull the way vendor
//! code does, and checks the three places it must show: the input register
//! firmware reads, `driver().pull` (what a world `gpio_net` counts), and a
//! push-captured own-drive channel.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::pins::{External, Pull};
use labwired_core::Bus;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bus(chip_file: &str) -> SystemBus {
    let chip = ChipDescriptor::from_file(root().join("configs/chips").join(chip_file)).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: pad-control\nchip: {chip_file}\nexternal_devices: []\nboard_io: []\n"
    ))
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.set_clock_gating_bypass(true);
    bus
}

/// Input register bit, `driver().pull`, and what the outside can still do.
fn assert_pad(bus: &mut SystemBus, port: &str, pin: u8, pull: Pull, what: &str) {
    let idx = bus.find_peripheral_index_by_name(port).unwrap();
    let pins = bus.peripherals[idx].dev.pins().unwrap();
    assert_eq!(
        pins.driver(pin).map(|d| d.pull),
        Some(pull),
        "{what}: driver().pull"
    );
    if let Some(rail) = pull.level() {
        assert_eq!(pins.input(pin), Some(rail), "{what}: input register");
        // The outside wins over the weak pull, and releasing hands it back.
        let change = bus
            .set_pad_external(idx, pin, External::Level(!rail))
            .unwrap();
        assert_eq!(change.after, !rail, "{what}: driven against the pull");
        let change = bus.set_pad_external(idx, pin, External::Released).unwrap();
        assert_eq!(change.after, rail, "{what}: released to the pull");
    }
}

#[test]
fn kinetis_portc_pcr_pulls_reach_gpioc() {
    let mut bus = bus("mkw41z4.yaml");
    // KW41Z reset: PCR MUX = 0 (disabled), no pull anywhere on GPIOC.
    assert_pad(&mut bus, "gpioc", 1, Pull::None, "reset");
    // PORTC_PCR1 = MUX 1 (GPIO) | PE | PS: pull-up.
    bus.write_u32(0x4004_B000 + 4, 0x0000_0103).unwrap();
    assert_pad(&mut bus, "gpioc", 1, Pull::Up, "PE|PS");
    // PS clear: pull-down.
    bus.write_u32(0x4004_B000 + 4, 0x0000_0102).unwrap();
    assert_pad(&mut bus, "gpioc", 1, Pull::Down, "PE");
    // MUX back to 0: the pin is disabled and the pull does not act.
    bus.write_u32(0x4004_B000 + 4, 0x0000_0003).unwrap();
    assert_pad(&mut bus, "gpioc", 1, Pull::None, "MUX 0");
}

#[test]
fn ra_pfs_pcr_pull_up_reaches_the_port() {
    let mut bus = bus("ra4m1.yaml");
    const PFS: u64 = 0x4004_0800;
    const P111PFS: u64 = PFS + 0x40 + 4 * 11;
    // Write-protected out of reset: the PCR store is dropped.
    bus.write_u32(P111PFS, 1 << 4).unwrap();
    assert_pad(&mut bus, "port1", 11, Pull::None, "PWPR locked");
    // R_BSP_PinAccessEnable, then the pull-up.
    bus.write_u8(PFS + 0x503, 0x00).unwrap();
    bus.write_u8(PFS + 0x503, 0x40).unwrap();
    bus.write_u32(P111PFS, 1 << 4).unwrap();
    assert_pad(&mut bus, "port1", 11, Pull::Up, "PCR");
    // Port 3 has its own PFS row: P111 does not leak into P311.
    assert_pad(&mut bus, "port3", 11, Pull::None, "other port");
    bus.write_u32(P111PFS, 0).unwrap();
    assert_pad(&mut bus, "port1", 11, Pull::None, "PCR cleared");
}

#[test]
fn imxrt_iomuxc_pad_ctl_pulls_reach_gpio2() {
    let mut bus = bus("imxrt1064.yaml");
    // SW_PAD_CTL_PAD_GPIO_B0_03 (GPIO2_IO03, Teensy 4 D13).
    const PAD: u64 = 0x401F_8000 + 0x32C + 4 * 3;
    // The reset value selects the keeper (PUE = 0): no pull.
    bus.write_u32(PAD, 0x0000_10B0).unwrap();
    assert_pad(&mut bus, "gpio2", 3, Pull::None, "keeper");
    // PKE | PUE | PUS = 10 (100k up).
    bus.write_u32(PAD, 0x0000_B0B0).unwrap();
    assert_pad(&mut bus, "gpio2", 3, Pull::Up, "100k up");
    // PUS = 00: 100k down.
    bus.write_u32(PAD, 0x0000_30B0).unwrap();
    assert_pad(&mut bus, "gpio2", 3, Pull::Down, "100k down");
    // A neighbouring pad register moves only its own pin.
    assert_pad(&mut bus, "gpio2", 4, Pull::None, "GPIO2_IO04");
}

/// A pull written in the pad-control block is a pad change: a probe on the
/// chip's own drive sees it as it happens (push capture), not only at the
/// next GPIO register write.
#[test]
fn a_pad_control_write_is_push_captured() {
    use labwired_core::logic_capture::{LogicSource, PadState};
    use labwired_core::system::cortex_m::configure_cortex_m;
    use labwired_core::{DebugControl, Machine};
    let mut bus = bus("imxrt1064.yaml");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let gpio2 = machine.bus.find_peripheral_index_by_name("gpio2").unwrap();
    machine.logic_watch(&[Some(LogicSource::driver(gpio2, 3))]);
    assert!(!machine.logic_poll_active(), "push capture armed");
    // A `b .` in RAM keeps the core busy so the run drains the tap.
    machine.bus.write_u16(0x2001_0000, 0xE7FE).unwrap();
    machine.cpu.pc = 0x2001_0000;
    machine.run(Some(2)).unwrap();
    machine
        .bus
        .write_u32(0x401F_8000 + 0x32C + 4 * 3, 0x0000_B0B0)
        .unwrap();
    machine.run(Some(2)).unwrap();
    let states = machine.logic_read_states(0).edges;
    assert!(
        states
            .iter()
            .any(|e| e.ch == 0 && e.state == PadState::WeakHigh),
        "the pull-up was pushed: {states:?}"
    );
}
