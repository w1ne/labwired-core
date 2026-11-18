// SPDX-License-Identifier: MIT
//! GPIO truth of the native DI8 adapter, using the existing declarative engine.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
fn bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    ))
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str("name: di8\nchip: stm32l476\nexternal_devices:\n- id: di8\n  type: sn74hc165_gpio\n  connection: gpio\n  config: {clock_pin: PB13, data_pin: PB14, latch_pin: PC6, inputs: 165}\n").unwrap();
    let mut b = SystemBus::from_config(&chip, &manifest).unwrap();
    b.write_u32(0x4002104c, 6).unwrap();
    b.write_u32(0x48000400, 1 << 26).unwrap();
    b.write_u32(0x48000800, 1 << 12).unwrap();
    b
}
fn clock(b: &mut SystemBus, high: bool) {
    b.write_u32(0x48000418, 1 << if high { 13 } else { 29 })
        .unwrap();
}
fn load(b: &mut SystemBus, high: bool) {
    b.write_u32(0x48000818, 1 << if high { 6 } else { 22 })
        .unwrap();
}
fn q(b: &SystemBus) -> bool {
    b.read_u32(0x48000410).unwrap() & (1 << 14) != 0
}
fn set(b: &mut SystemBus, pattern: u8) {
    for i in 0..8 {
        b.set_input(
            Some("di8"),
            &format!("ch{i}"),
            f64::from((pattern >> i) & 1),
        )
        .unwrap();
    }
}
fn read(b: &mut SystemBus) -> u8 {
    let mut result = 0;
    for _ in 0..8 {
        result = (result << 1) | u8::from(q(b));
        clock(b, true);
        clock(b, false);
    }
    result
}
#[test]
fn every_parallel_pattern_arrives_h_first_without_peripheral_ticks() {
    let mut b = bus();
    assert!(q(&b), "seeded0xA5 H is high while loading");
    for pattern in 0u16..256 {
        clock(&mut b, false);
        load(&mut b, false);
        set(&mut b, pattern as u8);
        load(&mut b, true);
        let at = b.current_cycle;
        assert_eq!(read(&mut b), pattern as u8);
        assert_eq!(b.current_cycle, at);
        for _ in 0..4 {
            assert!(!q(&b));
            clock(&mut b, true);
            clock(&mut b, false);
        }
    }
}
#[test]
fn low_load_is_transparent_and_clocks_do_not_shift() {
    let mut b = bus();
    load(&mut b, false);
    set(&mut b, 0);
    b.tick_peripherals_fully_forced();
    assert!(!q(&b));
    set(&mut b, 0x80);
    b.tick_peripherals_fully_forced();
    assert!(q(&b));
    for _ in 0..8 {
        clock(&mut b, true);
        assert!(q(&b));
        clock(&mut b, false);
        assert!(q(&b));
    }
}
#[test]
fn high_load_holds_the_word_across_live_input_changes() {
    let mut b = bus();
    load(&mut b, false);
    set(&mut b, 0xa5);
    load(&mut b, true);
    set(&mut b, 0x3c);
    assert_eq!(read(&mut b), 0xa5);
    load(&mut b, false);
    load(&mut b, true);
    assert_eq!(read(&mut b), 0x3c);
}
