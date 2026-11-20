// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! STM32L476 CMSIS dc04e41: EXTICR offsets08..14, three-bit source fields
//! spaced four bits apart; RCC_APB2ENR.SYSCFGEN bit0. Actual bus edge routing.
use labwired_config::{ChipDescriptor, PeripheralDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::declarative::GenericPeripheral;
use labwired_core::{Bus, Peripheral};
const SYSCFG: u64 = 0x40010000;
const EXTI: u64 = 0x40010400;
fn bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    ))
    .unwrap();
    let mut b = SystemBus::from_config(
        &chip,
        &SystemManifest {
            name: "l476-mux".into(),
            chip: "stm32l476".into(),
            ..Default::default()
        },
    )
    .unwrap();
    b.write_u32(0x4002104c, 3).unwrap();
    b.write_u32(0x48000000, 0).unwrap();
    b.write_u32(0x48000400, 0).unwrap();
    b
}
#[test]
fn exticr_retains_all_sixteen_sources_and_reserved_bits_stay_zero() {
    let mut b = bus();
    b.write_u32(SYSCFG + 8, 0x1111).unwrap();
    b.write_u32(0x40021060, 1).unwrap();
    assert_eq!(
        b.read_u32(SYSCFG + 8).unwrap(),
        0,
        "clock-disabled writes must not change the mux"
    );
    for offset in [8, 12, 16, 20] {
        b.write_u32(SYSCFG + offset, 0xffffffff).unwrap();
        assert_eq!(b.read_u32(SYSCFG + offset).unwrap(), 0x7777);
        b.write_u32(SYSCFG + offset, 0).unwrap();
        b.write_u8(SYSCFG + offset + 1, 0xff).unwrap();
        assert_eq!(b.read_u32(SYSCFG + offset).unwrap(), 0x7700);
    }
}
#[test]
fn each_line_respects_the_selected_port_edge_polarity_and_masked_pending_flag() {
    let mut b = bus();
    b.write_u32(0x40021060, 1).unwrap();
    b.write_u32(EXTI, 0).unwrap(); // masked interrupts still collect PR
    b.write_u32(EXTI + 8, 0).unwrap();
    b.write_u32(EXTI + 12, 0xffff).unwrap();
    for line in 0..16u8 {
        let offset = 8 + u64::from(line / 4) * 4;
        let shift = u32::from(line % 4) * 4;
        let value = b.read_u32(SYSCFG + offset).unwrap() | (1 << shift); // PB source
        b.write_u32(SYSCFG + offset, value).unwrap();
        assert!(SystemBus::drive_pin_input(
            &mut b,
            &format!("PA{line}"),
            true
        ));
        assert!(SystemBus::drive_pin_input(
            &mut b,
            &format!("PA{line}"),
            false
        ));
        assert_eq!(
            b.read_u32(EXTI + 20).unwrap() & (1 << line),
            0,
            "wrong port"
        );
        assert!(SystemBus::drive_pin_input(
            &mut b,
            &format!("PB{line}"),
            true
        ));
        assert_eq!(
            b.read_u32(EXTI + 20).unwrap() & (1 << line),
            0,
            "wrong edge"
        );
        assert!(SystemBus::drive_pin_input(
            &mut b,
            &format!("PB{line}"),
            false
        ));
        assert_ne!(
            b.read_u32(EXTI + 20).unwrap() & (1 << line),
            0,
            "selected falling edge"
        );
        b.write_u32(EXTI + 20, 1 << line).unwrap();
        assert_eq!(
            b.read_u32(EXTI + 20).unwrap() & (1 << line),
            0,
            "PR write-one clear"
        );
    }
}
#[test]
fn mux_is_opt_in_and_missing_or_overlapping_descriptor_fields_are_rejected() {
    let descriptor = PeripheralDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/peripherals/stm32l476/syscfg.yaml"
    ))
    .unwrap();
    assert_eq!(
        GenericPeripheral::new(descriptor.clone()).exti_line_source(6),
        None
    );
    let mux = GenericPeripheral::new(descriptor.clone())
        .with_exti_line_source()
        .unwrap();
    for line in 0..16 {
        assert_eq!(mux.exti_line_source(line), Some(0));
    }
    assert_eq!(mux.exti_line_source(16), None);
    let mut missing = descriptor.clone();
    for reg in &mut missing.registers {
        reg.fields.retain(|field| field.name != "EXTI6");
    }
    assert!(GenericPeripheral::new(missing)
        .with_exti_line_source()
        .is_err());
    let mut overlap = descriptor;
    let reg = overlap
        .registers
        .iter_mut()
        .find(|r| r.id == "EXTICR2")
        .unwrap();
    let first = reg.fields[0].bit_range;
    reg.fields[1].bit_range = first;
    assert!(GenericPeripheral::new(overlap)
        .with_exti_line_source()
        .is_err());
}
