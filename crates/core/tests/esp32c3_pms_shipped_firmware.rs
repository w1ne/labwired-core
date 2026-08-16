// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The SHIPPED BLE Pong image must arm the C3 PMS in the twin, with the same
//! configuration it programs on silicon — and must not trip it.
//!
//! # Why this exists next to `esp32c3_pms_violation`
//!
//! That file configures the PMS itself, the way `esp_mprot_set_prot()` does,
//! and proves the model reacts correctly. It cannot prove the thing that
//! actually matters: that REAL firmware, booting through the real mask ROM,
//! reaches memory-protection setup in the twin at all. If IDF's startup never
//! ran `esp_mprot_set_prot` here — because a thunk skipped it, or because
//! `esp_cpu_dbgr_is_attached()` read back as "debugger present" — the model
//! would sit disarmed forever and every assertion in the other file would be
//! describing a code path no lab ever enters.
//!
//! So this boots the frozen image that PANICKED ON SILICON with
//! `Guru Meditation Error: Core 0 panic'ed (Memory protection fault)` and reads
//! the PMS registers back out of `SENSITIVE`.
//!
//! # The independent corroboration
//!
//! The split line this firmware programs is asserted against `0x4039_0C00`,
//! which was derived SEPARATELY — from the app-image segments of that same
//! build (IRAM text ends at `0x4039_0A84`, `.data` starts at `0x3FC9_0C00`,
//! and `gp = .data + 0x800` confirms the base), not from anything this engine
//! computes. Two independent derivations of the same address agreeing is what
//! makes the split-line decode credible rather than merely self-consistent.
//!
//! # The regression rail
//!
//! `violations == 0`. The firmware runs with protection armed and LOCKED for
//! the whole window and never trips it, so the model is not faulting a lab
//! that works. If a future change makes this non-zero, a working lab is about
//! to start panicking in the browser.

// RELEASE-ONLY, for the same reason as `world_esp32c3_ble_pong`: a C3 mask-ROM
// boot far enough for IDF startup to configure memory protection is ~12M
// instructions, which is seconds in release and minutes in debug.
#![cfg(all(feature = "event-scheduler", not(debug_assertions)))]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32c3_rom::{
    build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
};
use labwired_core::boot::esp32s3_rom::RomImages;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::memory::ProgramImage;
use labwired_core::peripherals::esp32c3::pms::{decode_split_line, map_iram_to_dram};
use labwired_core::{Arch, Bus, Cpu, Machine};
use std::path::PathBuf;

const SENSITIVE: u64 = 0x600C_1000;
const INTC: u64 = 0x600C_2000;
/// `ETS_MEMPROT_ERR_INUM`.
const MEMPROT_INUM: u32 = 26;
const IRAM0_PMS_SOURCE: u64 = 56;
const DRAM0_PMS_SOURCE: u64 = 57;
const IRAM0_SRAM_LOW: u32 = 0x4038_0000;

/// The split address derived from the app image's own segments — see the
/// module docs. Everything below checks the firmware against THIS, not against
/// whatever the engine happens to produce.
const EXPECTED_SPLIT: u32 = 0x4039_0C00;

/// Long enough for IDF startup to finish configuring memory protection (it
/// arms at ~6.2M) plus a healthy margin of the sketch actually running.
const BOOT_INSTRUCTIONS: usize = 12_000_000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

const ESP_IMAGE_HEADER_LEN: usize = 24;
const ESP_IMAGE_MAGIC: u8 = 0xE9;

fn esp32c3_bootloader_image(flash: &[u8]) -> ProgramImage {
    assert_eq!(flash[0], ESP_IMAGE_MAGIC, "bad bootloader image magic");
    let segment_count = flash[1] as usize;
    let entry = u32::from_le_bytes(flash[4..8].try_into().unwrap()) as u64;
    let mut program = ProgramImage::new(entry, Arch::RiscV);
    let mut cursor = ESP_IMAGE_HEADER_LEN;
    for _ in 0..segment_count {
        let load_addr = u32::from_le_bytes(flash[cursor..cursor + 4].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(flash[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        cursor += 8;
        program.add_segment(load_addr, flash[cursor..cursor + len].to_vec());
        cursor += len;
    }
    program
}

/// The browser fast-start assembly for one C3 node, matching
/// `world_esp32c3_ble_pong::build_node`.
fn boot_ble_pong() -> Machine<RiscV> {
    let flash = std::fs::read(root().join("tests/fixtures/esp32c3-ble-pong-flash.bin"))
        .expect("read BLE Pong flash image");
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-ble-pong.yaml"))
            .expect("load ble-pong system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build ble-pong bus");

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).expect("read C3 IROM");
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).expect("read C3 DROM");
    assert!(inject_rom_regions(
        &mut bus,
        &RomImages {
            irom: irom.clone(),
            drom,
        },
    ));
    for (dst, bytes) in c3_rom_data_init_writes(&irom) {
        for (i, b) in bytes.iter().enumerate() {
            let _ = bus.write_u8(dst as u64 + i as u64, *b);
        }
    }

    let bootloader = esp32c3_bootloader_image(&flash);
    let mut machine = build_rom_boot_machine(
        bus,
        flash.clone(),
        RomBootOpts {
            pinned_efuse_mac: None,
            usb_serial_sink: None,
        },
        |c| c,
    );
    for segment in &bootloader.segments {
        if machine.bus.flash.load_from_segment(segment)
            || machine.bus.ram.load_from_segment(segment)
            || machine
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
        {
            continue;
        }
        for (i, byte) in segment.data.iter().enumerate() {
            machine
                .bus
                .write_u8(segment.start_addr + i as u64, *byte)
                .expect("load bootloader segment");
        }
    }
    let sp_top = (chip.ram.base + chip.ram.size) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(bootloader.entry_point as u32);
    let rec = machine.bus.max_safe_tick_interval();
    machine.config.peripheral_tick_interval = rec;
    machine.bus.config.peripheral_tick_interval = rec;
    machine.config.idle_fast_forward_enabled = true;
    machine
}

#[test]
fn shipped_ble_pong_image_arms_the_pms_exactly_as_silicon_does() {
    let mut m = boot_ble_pong();
    let mut armed_at = None;
    for i in 0..BOOT_INSTRUCTIONS {
        if m.step().is_err() {
            break;
        }
        if armed_at.is_none() && i % 100_000 == 0 && m.bus.esp32c3_pms_armed() {
            armed_at = Some(i);
        }
    }

    assert!(
        m.bus.esp32c3_pms_armed(),
        "DISARMED: the shipped BLE Pong image booted without arming the PMS. \
         Either IDF startup never reached esp_mprot_set_prot in the twin, or \
         the SENSITIVE writes are not reaching the model — in which case every \
         memory-protection assertion elsewhere describes a path no lab enters."
    );
    let armed_at = armed_at.expect("armed flag observed");
    assert!(
        armed_at < BOOT_INSTRUCTIONS,
        "PMS armed only at the very end of the window ({armed_at})"
    );

    let rd = |off: u64| m.bus.read_u32(SENSITIVE + off).expect("SENSITIVE readable");

    // ── The split lines, against the independently derived address ──────────
    for (name, off) in [
        ("main I/D", 0x094u64),
        ("IRAM line 0", 0x098),
        ("IRAM line 1", 0x09C),
    ] {
        assert_eq!(
            decode_split_line(rd(off), IRAM0_SRAM_LOW),
            Some(EXPECTED_SPLIT),
            "{name} split line ({:#010x}) must decode to the address derived from \
             the image's own segments",
            rd(off)
        );
    }
    for (name, off) in [("DRAM line 0", 0x0A0u64), ("DRAM line 1", 0x0A4)] {
        assert_eq!(
            decode_split_line(rd(off), 0x3FC8_0000),
            Some(map_iram_to_dram(EXPECTED_SPLIT)),
            "{name} must sit at the DRAM view of the same split"
        );
    }

    // ── Permissions: IDF's defaults, read back from the register file ───────
    let iram = rd(0x0B0);
    let iram_areas: Vec<u32> = (0..4).map(|a| (iram >> (a * 3)) & 0x7).collect();
    assert_eq!(
        iram_areas,
        vec![0b101, 0b101, 0b101, 0b000],
        "IRAM0 areas must be R|X, R|X, R|X, NONE ({iram:#010x}) — text is not \
         writable and the IRAM view of the data region is not executable"
    );
    let dram = rd(0x0C4);
    let dram_areas: Vec<u32> = (0..4).map(|a| (dram >> (a * 2)) & 0x3).collect();
    assert_eq!(
        dram_areas,
        vec![0b00, 0b11, 0b11, 0b11],
        "DRAM0 areas must be NONE, R|W, R|W, R|W ({dram:#010x}) — the IRAM text \
         region is unreachable from the data bus"
    );

    // ── Monitors on, and LOCKED (CONFIG_ESP_SYSTEM_MEMPROT_FEATURE_LOCK=y) ──
    assert_eq!(rd(0x0B8) & 0x2, 0x2, "IRAM0 monitor must be enabled");
    assert_eq!(rd(0x0CC) & 0x2, 0x2, "DRAM0 monitor must be enabled");
    for (name, off) in [
        ("split lines", 0x090u64),
        ("IRAM0 permissions", 0x0A8),
        ("IRAM0 monitor", 0x0B4),
        ("DRAM0 permissions", 0x0C0),
        ("DRAM0 monitor", 0x0C8),
    ] {
        assert_eq!(
            rd(off) & 1,
            1,
            "{name} lock must be set — this image ships with \
             CONFIG_ESP_SYSTEM_MEMPROT_FEATURE_LOCK=y, so protection cannot be \
             relaxed after startup"
        );
    }

    // ── Delivery: the firmware itself wired both sources to INUM 26 ─────────
    assert_eq!(
        m.bus.read_u32(INTC + IRAM0_PMS_SOURCE * 4).unwrap(),
        MEMPROT_INUM,
        "esp_mprot_set_intr_matrix must have routed the IRAM0 PMS source to \
         ETS_MEMPROT_ERR_INUM"
    );
    assert_eq!(
        m.bus.read_u32(INTC + DRAM0_PMS_SOURCE * 4).unwrap(),
        MEMPROT_INUM,
        "...and the DRAM0 PMS source to the same INUM"
    );
    assert_ne!(
        m.bus.read_u32(INTC + 0x104).unwrap() & (1 << MEMPROT_INUM),
        0,
        "CPU interrupt line 26 must be enabled, or a violation could never \
         reach the firmware's panic handler"
    );

    // ── The regression rail ─────────────────────────────────────────────────
    assert_eq!(
        m.bus.esp32c3_pms_violations(),
        0,
        "REGRESSION: the shipped BLE Pong image tripped the PMS model during \
         normal operation. This lab runs clean on silicon; a violation here \
         means the model is too strict and is about to panic a working lab."
    );
}
