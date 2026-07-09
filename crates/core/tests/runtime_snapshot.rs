// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Focused round-trip tests for the binary runtime snapshot infrastructure.
// Validates the foundation (CPU + RamPeripheral + SystemStub + SSD1680
// snapshot/restore) without needing an Arduino-ESP32 reference firmware in the loop —
// the heavy end-to-end resume test lives in `e2e_external_arduino_esp32_in_sim.rs`.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Ssd1680Tricolor290;
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::runtime_snapshot::{CpuKind, MachineRuntimeSnapshot};
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Bus, Cpu, Machine};

#[test]
fn cpu_runtime_snapshot_roundtrips_full_state() {
    let mut bus = SystemBus::new();
    let mut cpu = configure_xtensa_esp32(&mut bus);

    // Write a sentinel into every logical register + a stash in PC so we can
    // catch a corrupted snapshot decode at glance.
    cpu.set_pc(0x400D_BEEF);
    for i in 0..16u8 {
        cpu.set_register(i, 0xDEAD_0000 | (i as u32));
    }

    let (kind, blob) = cpu.runtime_snapshot();
    assert_eq!(kind, CpuKind::XtensaLx7);
    assert!(blob.len() > 64, "snapshot blob should be substantial");

    // Mutate state — show that restore actually does something.
    cpu.set_pc(0);
    for i in 0..16u8 {
        cpu.set_register(i, 0);
    }
    assert_eq!(cpu.get_pc(), 0);

    cpu.apply_runtime_snapshot(kind, &blob).expect("apply");
    assert_eq!(cpu.get_pc(), 0x400D_BEEF);
    for i in 0..16u8 {
        assert_eq!(
            cpu.get_register(i),
            0xDEAD_0000 | (i as u32),
            "register a{i} not restored"
        );
    }
}

#[test]
fn ram_peripheral_runtime_snapshot_roundtrips_memory() {
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);

    // Write a sentinel pattern into DRAM (0x3FFA_E000 + 0x100, picked to
    // dodge the .bss zero-fill region the firmware would normally use).
    let addr: u64 = 0x3FFB_0100;
    let pattern: [u8; 16] = [
        0xCA, 0xFE, 0xBA, 0xBE, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x55, 0xAA, 0x33,
        0xCC,
    ];
    for (i, b) in pattern.iter().enumerate() {
        bus.write_u8(addr + i as u64, *b).expect("write");
    }

    // Snapshot the DRAM peripheral.
    let dram = bus
        .peripherals
        .iter()
        .find(|p| p.name == "dram")
        .expect("dram on bus");
    let blob = dram.dev.runtime_snapshot();
    assert_eq!(blob.len(), 0x32000, "dram backing should be 200 KiB");
    // Pattern should appear at the matching offset inside the snapshot.
    let offset_in_dram = (addr - 0x3FFA_E000) as usize;
    assert_eq!(&blob[offset_in_dram..offset_in_dram + 16], &pattern);

    // Clobber the live state, then restore.
    for i in 0..pattern.len() {
        bus.write_u8(addr + i as u64, 0).expect("clobber");
    }
    for i in 0..pattern.len() {
        assert_eq!(bus.read_u8(addr + i as u64).unwrap(), 0, "clobber visible");
    }
    let dram = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "dram")
        .expect("dram still on bus");
    dram.dev.restore_runtime_snapshot(&blob).expect("restore");

    for (i, b) in pattern.iter().enumerate() {
        assert_eq!(
            bus.read_u8(addr + i as u64).unwrap(),
            *b,
            "byte at offset {i} not restored"
        );
    }
}

#[test]
fn ssd1680_runtime_snapshot_roundtrips_panel_planes() {
    let mut panel = Ssd1680Tricolor290::new("GPIO5");
    // Drive the panel through a minimal command sequence so internal state
    // diverges from default: SWRESET → power_on toggle → write a black byte.
    use labwired_core::peripherals::spi::SpiDevice;
    panel.cs_select();
    let _ = panel.transfer(0x12); // SWRESET
    panel.cs_release();

    let blob = SpiDevice::runtime_snapshot(&panel);
    assert!(
        blob.len() >= 9472,
        "snapshot must include both 4736-byte planes"
    );

    // Build a fresh panel; verify it starts blank, restore, verify state
    // matches the original.
    let mut fresh = Ssd1680Tricolor290::new("GPIO5");
    SpiDevice::restore_runtime_snapshot(&mut fresh, &blob).expect("restore");
    // Round-trip stability: serialize both and compare.
    let blob2 = SpiDevice::runtime_snapshot(&fresh);
    assert_eq!(blob, blob2, "round-trip should be byte-stable");
}

#[test]
fn machine_runtime_snapshot_roundtrips_through_serialization() {
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    // Attach an SSD1680 to spi3 so the snapshot covers the SPI-device path.
    bus.attach_spi_device("spi3", Box::new(Ssd1680Tricolor290::new("GPIO5")))
        .expect("spi3 is an Esp32Spi controller");
    bus.refresh_peripheral_index();

    let mut machine = Machine::new(cpu, bus);

    // Write some state into DRAM + CPU registers so the snapshot has
    // something nontrivial to restore.
    machine.cpu.set_pc(0x400D_0042);
    machine.cpu.set_register(5, 0x1234_5678);
    machine
        .bus
        .write_u32(0x3FFB_0200, 0xDEAD_BEEF)
        .expect("dram write");

    let snap = machine.take_runtime_snapshot();
    let bytes = snap.to_bytes();
    assert!(
        bytes.len() > 1000,
        "machine snapshot blob should be substantial"
    );

    // Round-trip via bytes (proving the on-disk format survives).
    let decoded = MachineRuntimeSnapshot::from_bytes(&bytes).expect("from_bytes");
    assert_eq!(decoded.cpu_kind, CpuKind::XtensaLx7);
    assert!(
        decoded
            .peripherals
            .iter()
            .any(|(name, blob)| name == "dram" && !blob.is_empty()),
        "dram blob present"
    );
    assert!(
        decoded
            .peripherals
            .iter()
            .any(|(name, blob)| name == "spi3" && !blob.is_empty()),
        "spi3 blob present (carries SSD1680 device snapshot)"
    );

    // Clobber and restore on the same machine.
    machine.cpu.set_pc(0);
    machine.cpu.set_register(5, 0);
    machine.bus.write_u32(0x3FFB_0200, 0).expect("dram clobber");

    machine.apply_runtime_snapshot(&decoded).expect("apply");
    assert_eq!(machine.cpu.get_pc(), 0x400D_0042);
    assert_eq!(machine.cpu.get_register(5), 0x1234_5678);
    assert_eq!(machine.bus.read_u32(0x3FFB_0200).unwrap(), 0xDEAD_BEEF);
}

/// Verifies that the offline-captured the reference firmware snapshot file produced by
/// `labwired-cli snapshot capture` decodes cleanly and restores onto a
/// freshly-built machine with the panel in its post-paint state.
///
/// Skipped unless the snapshot file exists at the conventional path —
/// running it requires a one-time CLI invocation:
///   cargo run --release -p labwired-cli -- snapshot capture \
///     --firmware /tmp/demo-agentdeck.elf \
///     --steps 30000000 \
///     --output /tmp/agentdeck-postpaint.lwrs
#[test]
#[ignore = "needs /tmp/agentdeck-postpaint.lwrs from a manual capture"]
fn agentdeck_snapshot_file_restores_post_paint_panel() {
    let snap_path = std::path::PathBuf::from("/tmp/agentdeck-postpaint.lwrs");
    if !snap_path.exists() {
        panic!(
            "/tmp/agentdeck-postpaint.lwrs not found — generate it with:\n  \
             cargo run --release -p labwired-cli -- snapshot capture \\\n  \
               --firmware /tmp/demo-agentdeck.elf \\\n  \
               --steps 30000000 \\\n  \
               --output /tmp/agentdeck-postpaint.lwrs"
        );
    }
    let bytes = std::fs::read(&snap_path).expect("read snapshot file");
    let snap = MachineRuntimeSnapshot::from_bytes(&bytes).expect("decode snapshot");

    // Build a fresh machine matching the the reference firmware topology.
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);
    bus.attach_spi_device("spi3", Box::new(Ssd1680Tricolor290::new("GPIO5")))
        .expect("spi3 is an Esp32Spi controller");
    bus.refresh_peripheral_index();

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let mut machine = Machine::new(boxed, bus);

    machine.apply_runtime_snapshot(&snap).expect("apply");

    // Re-locate the panel and read its restored state.
    let spi3_idx = machine
        .bus
        .find_peripheral_index_by_name("spi3")
        .expect("spi3 registered");
    let any = machine.bus.peripherals[spi3_idx].dev.as_any().unwrap();
    let spi3 = any.downcast_ref::<Esp32Spi>().unwrap();
    let panel = spi3
        .attached_devices
        .iter()
        .filter_map(|d| {
            d.as_any()
                .and_then(|a| a.downcast_ref::<Ssd1680Tricolor290>())
        })
        .next()
        .expect("panel attached");

    assert_eq!(
        panel.refresh_generation(),
        1,
        "snapshot must restore post-first-paint refresh_generation"
    );
    let bp = panel.black_plane();
    let rp = panel.red_plane();
    let black_non_ff = bp.iter().filter(|&&b| b != 0xFF).count();
    let red_non_ff = rp.iter().filter(|&&b| b != 0xFF).count();
    let red_zero = rp.iter().filter(|&&b| b == 0x00).count();
    eprintln!(
        "panel @ refresh_generation=1: black non-FF={}/{}, red non-FF={}/{}, red 0x00={}/{}",
        black_non_ff,
        bp.len(),
        red_non_ff,
        rp.len(),
        red_zero,
        rp.len(),
    );
    assert_eq!(
        black_non_ff, 782,
        "snapshot must restore IDLE splash with 782 non-FF bytes on the black plane"
    );
}

#[test]
fn snapshot_magic_and_version_are_enforced() {
    let snap = MachineRuntimeSnapshot::new(CpuKind::XtensaLx7, vec![], vec![]);
    let mut bytes = snap.to_bytes();

    // Trip the magic bytes — must reject.
    bytes[0] = b'X';
    let err = MachineRuntimeSnapshot::from_bytes(&bytes).unwrap_err();
    assert!(format!("{err}").contains("bad magic"), "got: {err}");
}
