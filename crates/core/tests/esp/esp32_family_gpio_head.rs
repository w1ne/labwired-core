// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Official GPIO data/enable/input heads on the four previously UART-only
//! Espressif board profiles: the window is reachable at the cited base, W1TS
//! sets the latch, and IN loops back an enabled output. The C5/C61 ENABLE/IN
//! offsets must not be the C3 map.

use crate::common;
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

fn bus(system_rel: &str) -> SystemBus {
    let (chip, manifest) = common::system(system_rel);
    SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build bus for {system_rel}: {e}"))
}

fn poke_output(
    bus: &mut SystemBus,
    base: u64,
    out: u64,
    out_w1ts: u64,
    enable_w1ts: u64,
    input: u64,
    mask: u32,
) {
    bus.write_u32(base + out_w1ts, mask)
        .unwrap_or_else(|e| panic!("OUT_W1TS: {e}"));
    bus.write_u32(base + enable_w1ts, mask)
        .unwrap_or_else(|e| panic!("ENABLE_W1TS: {e}"));
    let latch = bus
        .read_u32(base + out)
        .unwrap_or_else(|e| panic!("OUT: {e}"));
    let sampled = bus
        .read_u32(base + input)
        .unwrap_or_else(|e| panic!("IN: {e}"));
    assert_eq!(latch & mask, mask, "OUT latch at {out:#x}");
    assert_eq!(sampled & mask, mask, "IN loopback at {input:#x}");
}

#[test]
fn s2_devkitc_gpio43_tx_is_on_the_official_base_and_bank1() {
    let mut bus = bus("configs/systems/esp32s2-devkitc-1-n8r2.yaml");
    // GPIO43 = OUT1 bit 11. Base and offsets: S2 reg_base.h / gpio_reg.h.
    poke_output(&mut bus, 0x3F40_4000, 0x10, 0x14, 0x30, 0x40, 1 << 11);
}

#[test]
fn c5_devkitc_gpio11_tx_uses_enable_0x34_not_the_c3_map() {
    let mut bus = bus("configs/systems/esp32c5-devkitc-1-n8r8.yaml");
    let base = 0x6009_1000;
    // Writing the C3 ENABLE_W1TS offset must not enable GPIO11.
    bus.write_u32(base + 0x24, 1 << 11).unwrap();
    bus.write_u32(base + 0x08, 1 << 11).unwrap();
    assert_eq!(bus.read_u32(base + 0x20).unwrap(), 0);
    assert_eq!(bus.read_u32(base + 0x3C).unwrap(), 0);
    poke_output(&mut bus, base, 0x04, 0x08, 0x38, 0x64, 1 << 11);
}

#[test]
fn c61_devkitc_gpio10_rx_uses_the_c5_family_offsets() {
    let mut bus = bus("configs/systems/esp32c61-devkitc-1-n8r2.yaml");
    poke_output(&mut bus, 0x6009_1000, 0x04, 0x08, 0x38, 0x64, 1 << 10);
}

#[test]
fn p4_ev_board_gpio37_tx_is_on_the_official_hpperiph1_gpio_base() {
    let mut bus = bus("configs/systems/esp32p4x-c5-function-ev-board.yaml");
    // GPIO37 = OUT1 bit 5. Base = 0x500C_0000 + 0x2_0000.
    poke_output(&mut bus, 0x500E_0000, 0x10, 0x14, 0x30, 0x40, 1 << 5);
}
