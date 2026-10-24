// SPDX-License-Identifier: MIT
//! Opt-in byte foundation only. All guest descriptors/data/MMIO are authored
//! by the CPU; host MMIO controls are separate and not substituted for guests.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, memory::ProgramImage, Arch, Bus, Machine};

const DMA: u64 = 0x4100a000;
const TABLE: u32 = 0x20001000;
const WB: u32 = 0x20001200;
const SOURCE: u32 = 0x20002000;
const DESTINATION: u32 = 0x20002100;
const LAST_RAM: u32 = 0x2002ffff;

fn board() -> SystemBus {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chip =
        ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    // Test-local opt-in; never add the foundation to the production descriptor.
    chip.peripherals.push(
        serde_yaml::from_str(
            "id: dmac-byte\ntype: sam_dmac_byte\nbase_address: 0x4100a000\nsize: 1KB\nirq: 31\n",
        )
        .unwrap(),
    );
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

fn channel(index: usize) -> u64 {
    DMA + 0x40 + index as u64 * 16
}

fn host_descriptor(bus: &mut SystemBus, index: usize, control: u16, count: u16, next: u32) {
    let address = u64::from(TABLE) + index as u64 * 16;
    bus.write_u16(address, control).unwrap();
    bus.write_u16(address + 2, count).unwrap();
    bus.write_u32(address + 4, SOURCE + u32::from(count))
        .unwrap();
    bus.write_u32(address + 8, DESTINATION + u32::from(count))
        .unwrap();
    bus.write_u32(address + 12, next).unwrap();
}

#[test]
fn mmio_tables_modes_and_unsupported_descriptors_refuse_without_success() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chip =
        ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    chip.peripherals.push(serde_yaml::from_str(
        "id: unqualified-full-dmac\ntype: sam_dmac\nbase_address: 0x4100a000\nsize: 1KB\nirq: 31\n"
    ).unwrap());
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    assert!(
        SystemBus::from_config(&chip, &manifest).is_err(),
        "full SAM name must not fuzzy-resolve to STM32 DMA"
    );
    for (control, count, next) in [
        (0x0c08, 1, 0),
        (0x0d09, 1, 0),
        (0x0c09, 1, TABLE),
        (0x0c09, 0, 0),
    ] {
        let mut bus = board();
        host_descriptor(&mut bus, 0, control, count, next);
        bus.write_u8(u64::from(SOURCE), 0x59).unwrap();
        assert!(bus.write_u32(DMA + 0x34, TABLE + 1).is_err());
        bus.write_u32(DMA + 0x34, TABLE).unwrap();
        bus.write_u32(DMA + 0x38, WB).unwrap();
        assert!(bus.write_u32(channel(0), 13 << 8 | 2).is_err());
        bus.write_u32(channel(0), 2).unwrap();
        bus.write_u32(channel(0), 1).unwrap();
        assert_eq!(
            bus.read_u32(channel(0)).unwrap(),
            2,
            "enabled channel ignores SWRST"
        );
        bus.write_u32(DMA, 0x102).unwrap();
        assert!(
            bus.write_u32(DMA, 1).is_err(),
            "enabled global SWRST access error"
        );
        assert_eq!(bus.read_u32(DMA).unwrap(), 0x102);
        assert!(
            bus.write_u32(DMA, 1 << 16).is_err(),
            "CRC control is not admitted"
        );
        bus.write_u32(DMA, 0xf02).unwrap();
        assert_eq!(bus.read_u32(DMA).unwrap(), 0x102, "LVLEN enable protection");
        bus.write_u32(DMA + 0x34, TABLE + 0x100).unwrap();
        assert_eq!(bus.read_u32(DMA + 0x34).unwrap(), TABLE);
        bus.write_u32(DMA + 0x10, 1).unwrap();
        bus.tick_peripherals();
        assert!(
            bus.read_u8(channel(0) + 14).is_err(),
            "explicit refusal, not invented flags"
        );
        assert_eq!(bus.read_u8(u64::from(DESTINATION)).unwrap(), 0);
        let snapshot = bus
            .peripherals
            .iter()
            .find(|p| p.name == "dmac-byte")
            .unwrap()
            .dev
            .snapshot();
        assert!(snapshot["refusal"].as_str().is_some());
        assert_eq!(snapshot["channels"][0]["flags"], 0);
        bus.write_u32(DMA, 0).unwrap();
        bus.write_u32(DMA, 1).unwrap();
        assert_eq!(bus.read_u8(channel(0) + 14).unwrap(), 0);
    }
    // Table/data targets that generically return Ok are not RAM capabilities.
    // Include stub/gated MMIO, self-MMIO while the model is swapped out for a
    // bus-assisted tick, flash and extra mappings. No byte or writeback commits.
    for (target, address) in [
        ("table", DMA as u32),
        ("table", 0x41000000),
        ("table", 0x100),
        ("writeback", DMA as u32),
        ("writeback", 0x41000000),
        ("writeback", 0x43000000),
        ("writeback", 0x53000000),
        ("writeback", 0x100),
        ("writeback", 0x60001000),
        ("source", 0x100),
        ("source", 0x41000000),
        ("destination", DMA as u32),
        ("destination", 0x43000000),
        ("destination", 0x53000000),
        ("destination", 0x100),
    ] {
        let mut bus = board();
        bus.ns_alias_offset = Some(0u64.wrapping_sub(0x10000000));
        // The mapped extent begins halfway through a writeback descriptor;
        // checking only its first address would miss it.
        bus.extra_mem
            .push(labwired_core::memory::LinearMemory::new(8, 0x60001008));
        host_descriptor(&mut bus, 0, 0x0c09, 1, 0);
        bus.write_u8(u64::from(SOURCE), 0x59).unwrap();
        bus.write_u32(0x100, 0xaabbccdd).unwrap();
        if target == "source" || target == "destination" {
            let field = if target == "source" { 4 } else { 8 };
            bus.write_u32(u64::from(TABLE) + field, address + 1)
                .unwrap();
        }
        bus.write_u32(DMA + 0x34, if target == "table" { address } else { TABLE })
            .unwrap();
        bus.write_u32(DMA + 0x38, if target == "writeback" { address } else { WB })
            .unwrap();
        bus.write_u32(channel(0), 2).unwrap();
        bus.write_u32(DMA, 0x102).unwrap();
        bus.write_u32(DMA + 0x10, 1).unwrap();
        bus.tick_peripherals();
        assert!(
            bus.read_u8(channel(0) + 14).is_err(),
            "{target} {address:x}"
        );
        assert_eq!(bus.read_u8(u64::from(DESTINATION)).unwrap(), 0);
        assert_eq!(bus.read_u32(u64::from(WB)).unwrap(), 0);
        assert_eq!(bus.read_u32(0x100).unwrap(), 0xaabbccdd);
        assert_eq!(bus.extra_mem.last().unwrap().data, vec![0; 8]);
        let snapshot = bus
            .peripherals
            .iter()
            .find(|p| p.name == "dmac-byte")
            .unwrap()
            .dev
            .snapshot();
        assert!(snapshot["refusal"].as_str().is_some());
        assert_eq!(snapshot["channels"][0]["flags"], 0);
        assert_eq!(snapshot["channels"][0]["pending"], false);
    }
}

#[test]
fn shared_irq_clear_preserves_other_channel_and_nonconsuming_peek() {
    let mut bus = board();
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus.write_u8(u64::from(SOURCE), 0x67).unwrap();
    bus.write_u32(DMA + 0x34, TABLE).unwrap();
    bus.write_u32(DMA + 0x38, WB).unwrap();
    for index in [4, 31] {
        host_descriptor(&mut bus, index, 0x0c09, 1, 0);
        bus.write_u8(channel(index) + 13, 2).unwrap();
        bus.write_u32(channel(index), 2).unwrap();
    }
    bus.write_u32(DMA, 0x102).unwrap();
    bus.write_u32(DMA + 0x10, (1 << 4) | (1 << 31)).unwrap();
    bus.tick_peripherals();
    assert_eq!(bus.read_u32(0xe000e204).unwrap() & 8, 8);
    assert_eq!(bus.read_u32(DMA + 0x24).unwrap(), (1 << 4) | (1 << 31));
    let dev = &bus
        .peripherals
        .iter()
        .find(|p| p.name == "dmac-byte")
        .unwrap()
        .dev;
    assert_eq!(dev.peek(0x8e), Some(2));
    bus.write_u8(channel(4) + 14, 2).unwrap();
    assert_eq!(bus.read_u32(0xe000e204).unwrap() & 8, 8);
    bus.write_u8(channel(31) + 14, 2).unwrap();
    assert_eq!(bus.read_u32(0xe000e204).unwrap() & 8, 0);
}

fn guest(index: usize, mode: &str) -> ProgramImage {
    let mut ops = Vec::<u16>::new();
    let mut literals = Vec::<(usize, u8, u32)>::new();
    let load = |ops: &mut Vec<u16>, literals: &mut Vec<(usize, u8, u32)>, rt, value| {
        literals.push((ops.len(), rt, value));
        ops.push(0);
    };
    let store = |ops: &mut Vec<u16>, literals: &mut Vec<(usize, u8, u32)>, addr, value, byte| {
        load(ops, literals, 0, addr);
        load(ops, literals, 1, value);
        ops.push(if byte { 0x7001 } else { 0x6001 });
    };
    ops.push(0xb672); // CPSID i, before programming the actual peripheral/NVIC.
    let c = channel(index) as u32;
    let descriptor = TABLE + index as u32 * 16;
    let writeback = WB + index as u32 * 16;
    let irq = 31 + index.min(4) as u32;
    for (address, value) in [
        (0xe000ed08, 0x4000),
        (0x20000000, 0),
        (0x20000004, 0),
        (0x20000008, 0),
        (0x2000000c, 0),
    ] {
        store(&mut ops, &mut literals, address, value, false);
    }
    for offset in (0..64).step_by(4) {
        store(&mut ops, &mut literals, SOURCE + offset, 0x44332211, false);
        store(&mut ops, &mut literals, DESTINATION + offset, 0, false);
    }
    store(&mut ops, &mut literals, LAST_RAM, 0x5a, true);
    let src_end = if mode == "fixed-source" {
        SOURCE
    } else if mode == "read-error" {
        LAST_RAM + 64
    } else {
        SOURCE + 64
    };
    let dst_end = if mode == "fixed-destination" {
        DESTINATION
    } else if mode == "write-error" {
        LAST_RAM + 64
    } else {
        DESTINATION + 64
    };
    for (address, value) in [
        (
            descriptor,
            64 << 16
                | match mode {
                    "noact" => 0x0c01,
                    "fixed-source" => 0x0809,
                    "fixed-destination" => 0x0409,
                    _ => 0x0c09,
                },
        ),
        (descriptor + 4, src_end),
        (descriptor + 8, dst_end),
        (descriptor + 12, 0),
        (writeback, 0xaaaaaaaa),
        (writeback + 4, 0xaaaaaaaa),
        (writeback + 8, 0xaaaaaaaa),
        (writeback + 12, 0xaaaaaaaa),
        (DMA as u32 + 0x34, TABLE),
        (
            DMA as u32 + 0x38,
            if mode == "writeback-error" {
                0x60000000
            } else {
                WB
            },
        ),
        (c, if mode == "channel-off" { 0 } else { 2 }),
        (
            DMA as u32,
            match mode {
                "global-off" => 0x100,
                "level-off" => 2,
                _ => 0x102,
            },
        ),
    ] {
        store(&mut ops, &mut literals, address, value, false);
    }
    store(
        &mut ops,
        &mut literals,
        c + 13,
        if mode == "inten" { 0 } else { 3 },
        true,
    );
    if mode != "nvic" {
        store(
            &mut ops,
            &mut literals,
            0xe000e100 + (irq / 32) * 4,
            1 << (irq % 32),
            false,
        );
    }
    if mode != "no-trigger" {
        store(
            &mut ops,
            &mut literals,
            DMA as u32 + 0x10,
            1 << index,
            false,
        );
    }
    if mode == "reset" {
        store(&mut ops, &mut literals, DMA as u32, 0, false);
        store(&mut ops, &mut literals, DMA as u32, 1, false);
    }
    if mode != "primask" {
        ops.push(0xb662);
    }
    store(&mut ops, &mut literals, 0x20000000, 0xcafebabe, false);
    ops.push(0xe7fe);
    let handler = 0x4200 + ops.len() as u32 * 2;
    load(&mut ops, &mut literals, 0, 0x20000004);
    ops.extend([0x6801, 0x3101, 0x6001]);
    store(&mut ops, &mut literals, 0x20000008, irq, false);
    load(&mut ops, &mut literals, 0, c + 14);
    ops.push(0x7801); // Actual flags before acknowledgement.
    load(&mut ops, &mut literals, 0, 0x2000000c);
    ops.push(0x6001);
    store(&mut ops, &mut literals, c + 14, 3, true);
    ops.push(0x4770);
    if ops.len() & 1 != 0 {
        ops.push(0xbf00);
    }
    let mut code: Vec<u8> = ops.iter().flat_map(|op| op.to_le_bytes()).collect();
    for (position, rt, value) in literals {
        let pc = ((position * 2 + 4) & !3) as u32;
        let delta = code.len() as u32 - pc;
        assert_eq!(delta & 3, 0);
        assert!(delta <= 1020);
        let op = 0x4800 | (u16::from(rt) << 8) | (delta / 4) as u16;
        code[position * 2..position * 2 + 2].copy_from_slice(&op.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    let mut image = vec![0; 0x200];
    image[..4].copy_from_slice(&0x20004000u32.to_le_bytes());
    image[4..8].copy_from_slice(&0x4201u32.to_le_bytes());
    let vector = (16 + irq as usize) * 4;
    image[vector..vector + 4].copy_from_slice(&(handler | 1).to_le_bytes());
    image.extend(code);
    let mut program = ProgramImage::new(0x4201, Arch::Arm);
    program.add_segment(0x4000, image);
    program
}

#[test]
fn authored_descriptor_guests_transfer_or_refuse_completion_and_return_from_correct_irq() {
    for index in [0, 1, 2, 3, 4, 31] {
        for mode in [
            "complete",
            "noact",
            "fixed-source",
            "fixed-destination",
            "primask",
            "nvic",
            "inten",
            "no-trigger",
            "global-off",
            "level-off",
            "channel-off",
            "reset",
            "read-error",
            "write-error",
            "writeback-error",
        ] {
            let mut bus = board();
            let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            let mut machine = Machine::new(cpu, bus);
            machine.load_firmware(&guest(index, mode)).unwrap();
            let mut entries = 0;
            let mut previous = 0;
            let mut returns = 0;
            let irq = 31 + index.min(4) as u32;
            for _ in 0..2000 {
                machine.step().unwrap();
                let active = machine.cpu.active_exception;
                if active != 0 {
                    assert_eq!(active, 16 + irq);
                }
                entries += usize::from(active != 0 && previous == 0);
                returns += usize::from(active == 0 && previous != 0);
                previous = active;
            }
            let delivered = matches!(
                mode,
                "complete"
                    | "fixed-source"
                    | "fixed-destination"
                    | "read-error"
                    | "write-error"
                    | "writeback-error"
            );
            let error = mode.ends_with("error");
            assert_eq!(
                entries,
                usize::from(delivered),
                "channel={index} mode={mode}"
            );
            assert_eq!(returns, usize::from(delivered));
            assert_eq!(machine.cpu.active_exception, 0);
            assert_eq!(machine.bus.read_u32(0x20000000).unwrap(), 0xcafebabe);
            assert_eq!(
                machine.bus.read_u32(0x20000004).unwrap(),
                u32::from(delivered)
            );
            assert_eq!(
                machine.bus.read_u32(0x20000008).unwrap(),
                if delivered { irq } else { 0 }
            );
            assert_eq!(
                machine.bus.read_u32(0x2000000c).unwrap(),
                if delivered {
                    if error {
                        1
                    } else {
                        2
                    }
                } else {
                    0
                }
            );
            assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
            assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
            let writeback = u64::from(WB) + index as u64 * 16;
            if matches!(
                mode,
                "complete"
                    | "noact"
                    | "fixed-source"
                    | "fixed-destination"
                    | "primask"
                    | "nvic"
                    | "inten"
            ) {
                for byte in 0..64 {
                    assert_eq!(
                        machine.bus.read_u8(u64::from(DESTINATION) + byte).unwrap(),
                        if mode == "fixed-source" {
                            0x11
                        } else if mode == "fixed-destination" {
                            if byte == 0 {
                                0x44
                            } else {
                                0
                            }
                        } else {
                            [0x11, 0x22, 0x33, 0x44][byte as usize % 4]
                        }
                    );
                }
                assert_eq!(machine.bus.read_u16(writeback + 2).unwrap(), 0);
                assert_eq!(machine.bus.read_u16(writeback).unwrap() & 1, 0);
                assert_eq!(
                    machine.bus.read_u32(writeback + 4).unwrap(),
                    if mode == "fixed-source" {
                        SOURCE
                    } else {
                        SOURCE + 64
                    }
                );
                assert_eq!(
                    machine.bus.read_u32(writeback + 8).unwrap(),
                    if mode == "fixed-destination" {
                        DESTINATION
                    } else {
                        DESTINATION + 64
                    }
                );
            } else if matches!(mode, "read-error" | "write-error") {
                assert_eq!(
                    machine.bus.read_u16(writeback + 2).unwrap(),
                    63,
                    "failed second beat must not decrement"
                );
                assert_eq!(machine.bus.read_u16(writeback).unwrap() & 1, 0);
                if mode == "read-error" {
                    assert_eq!(machine.bus.read_u8(u64::from(DESTINATION)).unwrap(), 0x5a);
                    assert_eq!(machine.bus.read_u8(u64::from(DESTINATION) + 1).unwrap(), 0);
                } else {
                    assert_eq!(machine.bus.read_u8(u64::from(LAST_RAM)).unwrap(), 0x11);
                }
            } else if mode == "writeback-error" {
                assert_eq!(machine.bus.read_u8(u64::from(DESTINATION)).unwrap(), 0x11);
                assert_eq!(machine.bus.read_u8(u64::from(DESTINATION) + 1).unwrap(), 0);
            } else if mode == "reset" {
                let remaining = machine.bus.read_u16(writeback + 2).unwrap();
                assert!(
                    (1..64).contains(&remaining),
                    "reset control must interrupt actual progress"
                );
                assert_eq!(machine.bus.read_u8(u64::from(DESTINATION)).unwrap(), 0x11);
                assert_eq!(machine.bus.read_u8(u64::from(DESTINATION) + 63).unwrap(), 0);
            } else {
                assert_eq!(machine.bus.read_u8(u64::from(DESTINATION)).unwrap(), 0);
                assert_eq!(machine.bus.read_u32(writeback).unwrap(), 0xaaaaaaaa);
            }
            let flags = machine.bus.read_u8(channel(index) + 14).unwrap();
            if delivered || matches!(mode, "noact" | "primask" | "nvic" | "inten" | "reset") {
                assert_eq!(
                    machine.bus.read_u32(channel(index)).unwrap() & 2,
                    0,
                    "terminal channel must be disabled"
                );
            }
            assert_eq!(
                flags,
                if matches!(mode, "primask" | "nvic" | "inten") {
                    2
                } else {
                    0
                }
            );
            if delivered || mode == "reset" {
                assert_eq!(
                    machine
                        .bus
                        .read_u32(0xe000e200 + (irq / 32) as u64 * 4)
                        .unwrap()
                        & (1 << (irq % 32)),
                    0
                );
            }
            let witness = machine.bus.read_u32(0x2000000c).unwrap();
            let remaining = machine.bus.read_u16(writeback + 2).unwrap();
            println!("SAM_DMAC_BYTE channel={index} mode={mode} entries={entries} returns={returns} flags={flags} witness={witness} remaining={remaining} exception={}", machine.cpu.active_exception);
        }
    }
}
