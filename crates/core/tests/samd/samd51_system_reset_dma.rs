// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! SAMD51 Stage C: AIRCR.SYSRESETREQ resets the SAM peripheral models, the
//! opt-in USB device banks build only on their fixed instance, and the opt-in
//! board DMAC moves TC3-overflow-paced half-word samples into DAC DATA0.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, memory::ProgramImage, system::cortex_m::configure_cortex_m, Arch, Bus, Machine,
};

const MCLK: u64 = 0x40000800;
const GCLK: u64 = 0x40001c00;
const TC3: u64 = 0x4101c000;
const DAC: u64 = 0x43002400;
const USB: u64 = 0x41000000;
const DMA: u64 = 0x4100a000;
const TABLE: u32 = 0x20001000;
const WB: u32 = 0x20001200;
const SAMPLES: u32 = 0x20002000;

const USB_CFG: &str = "id: usb-banks\ntype: samd51_usb_device_banks\nbase_address: 0x41000000\nsize: 1KB\nirq: 80\nclock:\n  - { controller: \"mclk\", reg: \"AHBMASK\", bit: 10 }\n  - { controller: \"mclk\", reg: \"APBBMASK\", bit: 0 }\n";
const DMAC_CFG: &str = "id: dmac-board\ntype: samd51_dmac_board\nbase_address: 0x4100a000\nsize: 1KB\nirq: 31\nclock: { controller: \"mclk\", reg: \"AHBMASK\", bit: 9 }\n";

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Metro M4 with the test-local opt-in models swapped in for their stubs.
fn chip(extra: &[&str]) -> (ChipDescriptor, SystemManifest) {
    let mut chip = ChipDescriptor::from_file(root().join("configs/chips/atsamd51.yaml")).unwrap();
    for cfg in extra {
        let p: labwired_config::PeripheralConfig = serde_yaml::from_str(cfg).unwrap();
        chip.peripherals
            .retain(|q| q.base_address != p.base_address);
        chip.peripherals.push(p);
    }
    let manifest = SystemManifest::from_file(root().join("configs/systems/metro-m4.yaml")).unwrap();
    (chip, manifest)
}

/// A machine parked on an idle `b .` loop, so the host drives MMIO.
///
/// Vectors live at flash base 0: `configure_cortex_m` installs an SCB, so
/// `load_firmware` / SYSRESETREQ both reload MSP/PC from VTOR (default 0).
/// Loading the table at 0x4000 left address 0 erased (0xFFFFFFFF) and the
/// first `step` faulted with MemoryViolation(0xFFFFFFFE).
fn idle_machine(extra: &[&str]) -> Machine<labwired_core::cpu::cortex_m::CortexM> {
    let (chip, manifest) = chip(extra);
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let mut bytes = vec![0u8; 0x104];
    bytes[0..4].copy_from_slice(&0x20004000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x0000_0101u32.to_le_bytes()); // Thumb entry at 0x100
    bytes[0x100..0x102].copy_from_slice(&0xe7feu16.to_le_bytes()); // b .
    let mut image = ProgramImage::new(0x101, Arch::Arm);
    image.add_segment(0x0000, bytes);
    machine.load_firmware(&image).unwrap();
    machine
}

fn set_bits(bus: &mut SystemBus, addr: u64, bits: u32) {
    let v = bus.read_u32(addr).unwrap();
    bus.write_u32(addr, v | bits).unwrap();
}

#[test]
fn usb_device_banks_build_only_on_their_fixed_instance() {
    let (c, m) = chip(&[USB_CFG]);
    let bus = SystemBus::from_config(&c, &m).unwrap();
    assert!(bus
        .peripherals
        .iter()
        .any(|p| p.name == "usb-banks" && p.base == USB));
    for bad in [
        USB_CFG.replace("irq: 80", "irq: 81"),
        USB_CFG.replace(
            "  - { controller: \"mclk\", reg: \"APBBMASK\", bit: 0 }\n",
            "",
        ),
    ] {
        let (c, m) = chip(&[bad.as_str()]);
        assert!(SystemBus::from_config(&c, &m).is_err(), "{bad}");
    }
}

#[test]
fn sysresetreq_returns_sam_models_to_their_reset_state() {
    let mut machine = idle_machine(&[USB_CFG]);
    let bus = &mut machine.bus;
    set_bits(bus, MCLK + 0x10, 1 << 10); // AHBMASK: USB
    set_bits(bus, MCLK + 0x18, 1 | 1 << 14); // APBBMASK: USB, TC3
    let pchctrl26 = GCLK + 0x80 + 26 * 4;
    let before = (
        bus.read_u32(pchctrl26).unwrap(),
        bus.read_u8(TC3).unwrap(),
        bus.read_u8(DAC).unwrap(),
        bus.read_u8(USB).unwrap(),
    );
    bus.write_u32(pchctrl26, 0x40).unwrap(); // CHEN, GEN0
    bus.write_u32(TC3, 2).unwrap(); // CTRLA.ENABLE
    bus.write_u8(DAC, 2).unwrap(); // CTRLA.ENABLE
    bus.write_u8(USB, 0x04).unwrap(); // CTRLA.RUNSTDBY
    assert_ne!(bus.read_u32(pchctrl26).unwrap(), before.0);
    assert_ne!(bus.read_u8(TC3).unwrap() & 2, before.1 & 2);
    assert_ne!(bus.read_u8(DAC).unwrap() & 2, before.2 & 2);
    assert_ne!(bus.read_u8(USB).unwrap() & 4, before.3 & 4);

    machine.bus.write_u32(0xE000_ED0C, 0x05FA_0004).unwrap(); // SYSRESETREQ
    machine.step().expect("step into the reset boundary");

    let bus = &mut machine.bus;
    set_bits(bus, MCLK + 0x10, 1 << 10);
    set_bits(bus, MCLK + 0x18, 1 | 1 << 14);
    assert_eq!(bus.read_u32(pchctrl26).unwrap(), before.0, "GCLK PCHCTRL26");
    assert_eq!(bus.read_u8(TC3).unwrap() & 2, before.1 & 2, "TC3 enable");
    assert_eq!(bus.read_u8(DAC).unwrap() & 2, before.2 & 2, "DAC enable");
    assert_eq!(bus.read_u8(USB).unwrap() & 4, before.3 & 4, "USB RUNSTDBY");
}

#[test]
fn board_dmac_paces_dac_samples_on_tc3_overflow() {
    let mut machine = idle_machine(&[DMAC_CFG]);
    let bus = &mut machine.bus;
    set_bits(bus, MCLK + 0x10, 1 << 9); // AHBMASK: DMAC
    set_bits(bus, MCLK + 0x18, 1 << 14); // APBBMASK: TC3
    bus.write_u32(GCLK + 0x80 + 26 * 4, 0x40).unwrap(); // TC2/TC3 on GEN0
    let samples = [0x123u16, 0x456, 0x789];
    for (i, s) in samples.iter().enumerate() {
        bus.write_u16(u64::from(SAMPLES) + 2 * i as u64, *s)
            .unwrap();
    }
    // VALID | BEATSIZE=HWORD | SRCINC; SRCADDR is the block end.
    let t = u64::from(TABLE);
    bus.write_u16(t, 0x0501).unwrap();
    bus.write_u16(t + 2, samples.len() as u16).unwrap();
    bus.write_u32(t + 4, SAMPLES + 2 * samples.len() as u32)
        .unwrap();
    bus.write_u32(t + 8, (DAC + 0x10) as u32).unwrap();
    bus.write_u32(t + 12, 0).unwrap();
    bus.write_u32(DMA + 0x34, TABLE).unwrap();
    bus.write_u32(DMA + 0x38, WB).unwrap();
    bus.write_u32(DMA, 0x0f02).unwrap(); // LVLEN0..3, DMAENABLE
    bus.write_u32(DMA + 0x40, (0x35 << 8) | (2 << 20) | 2)
        .unwrap(); // TC3 OVF, BURST
                   // TC3: COUNT16, MFRQ with TOP = CC0, DIV1.
    bus.write_u8(TC3 + 0x0c, 1).unwrap();
    bus.write_u16(TC3 + 0x1c, 99).unwrap();
    bus.write_u32(TC3, 2).unwrap();

    let mut seen = Vec::new();
    for _ in 0..200_000 {
        machine.step().unwrap();
        let v = machine.bus.read_u16(DAC + 0x1c).unwrap();
        if seen.last() != Some(&v) {
            seen.push(v);
        }
        if machine.bus.read_u8(DMA + 0x4e).unwrap() & 2 != 0 {
            break;
        }
    }
    assert!(
        machine.bus.read_u8(DMA + 0x4e).unwrap() & 2 != 0,
        "TCMPL after the block; DAC saw {seen:x?}"
    );
    for s in samples {
        assert!(seen.contains(&s), "DAC DATA0 got {s:#x}: {seen:x?}");
    }
}
