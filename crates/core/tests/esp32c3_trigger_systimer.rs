// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 debug triggers and the SYSTIMER on the bare-ELF fast boot.
//!
//! Two gaps used to keep an ordinary esp-hal application off the C3 fast boot:
//!
//! * esp-hal's default stack guard programs trigger 0 as a store watchpoint on
//!   `__stack_chk_guard` (`tselect`/`tcontrol`/`tdata1`/`tdata2`). The RISC-V
//!   model had no trigger CSRs, so the first `csrw tselect` was an illegal
//!   instruction and the app died before `main`.
//! * The chip YAML's SYSTIMER was a register stub, so esp-hal's `Delay` (UNIT0
//!   update / VALUE_VALID handshake, then a read of the snapshot) never saw
//!   time pass. Only the mask-ROM boot installed the real timer.
//!
//! The fixture is an unmodified-config esp-hal 1.1 build (stack guard on) that
//! times three `Delay::delay_millis(2)` calls with `Instant` and then writes the
//! guard word itself; silicon catches that write with the watchpoint and
//! esp-hal's exception handler names it. See
//! `tests/fixtures/esp32c3-esp-hal-guard-delay.md`.
//!
//! The hand-assembled test drives the same two mechanisms without esp-hal, so a
//! failure there points at the engine rather than at the fixture.

mod common;

use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use labwired_core::{Bus, DebugControl, Machine};
use std::time::Duration;

fn c3_rom_blobs() -> BlobMap {
    let mut blobs = BlobMap::new();
    blobs.insert(
        "esp32c3_irom".into(),
        common::committed("crates/core/roms/esp32c3/esp32c3_rom.bin"),
    );
    blobs.insert(
        "esp32c3_drom".into(),
        common::committed("crates/core/roms/esp32c3/esp32c3_drom.bin"),
    );
    blobs
}

#[test]
fn esp_hal_stack_guard_and_delay_behave_like_silicon_on_the_fast_boot() {
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let fw = common::committed("tests/fixtures/esp32c3-esp-hal-guard-delay.elf");
    let blobs = c3_rom_blobs();
    let mut s = Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap();

    let caught = s.expect(
        "Detected a write to the main stack's guard value",
        Duration::from_millis(500),
    );
    let console = s.uart_transcript();
    caught.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{console}"));

    // The guard programming did not trap: main ran.
    assert!(console.contains("C3 GUARD BOOT"), "{console}");
    // Each 2 ms delay took 2 ms of SYSTIMER time: the counter advances at
    // 16 MHz against the 160 MHz core, so `Instant` and `Delay` agree with the
    // simulated clock (a frozen counter would hang inside the first delay).
    let elapsed: Vec<u64> = console
        .lines()
        .filter_map(|l| l.split("elapsed_us=").nth(1))
        .map(|v| v.trim().parse().unwrap())
        .collect();
    assert_eq!(elapsed.len(), 3, "{console}");
    for us in &elapsed {
        assert!(
            (2_000..2_100).contains(us),
            "a 2 ms Delay measured {us} us\n{console}"
        );
    }
    // The watched store trapped before it retired.
    assert!(console.contains("C3 WRITING GUARD"), "{console}");
    assert!(!console.contains("GUARD WRITE NOT CAUGHT"), "{console}");
}

// --- hand-assembled -------------------------------------------------------

fn lui(rd: u32, imm20: u32) -> u32 {
    (imm20 << 12) | (rd << 7) | 0x37
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    (((imm as u32) & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13
}
fn and(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (rs2 << 20) | (rs1 << 15) | (0b111 << 12) | (rd << 7) | 0x33
}
fn csrrw(rd: u32, csr: u32, rs1: u32) -> u32 {
    (csr << 20) | (rs1 << 15) | (0b001 << 12) | (rd << 7) | 0x73
}
fn csrrs(rd: u32, csr: u32, rs1: u32) -> u32 {
    (csr << 20) | (rs1 << 15) | (0b010 << 12) | (rd << 7) | 0x73
}
fn sw(rs2: u32, imm: i32, rs1: u32) -> u32 {
    let imm = imm as u32;
    (((imm >> 5) & 0x7F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (0b010 << 12)
        | ((imm & 0x1F) << 7)
        | 0x23
}
fn lw(rd: u32, imm: i32, rs1: u32) -> u32 {
    (((imm as u32) & 0xFFF) << 20) | (rs1 << 15) | (0b010 << 12) | (rd << 7) | 0x03
}
fn branch(funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    (((imm >> 12) & 1) << 31)
        | (((imm >> 5) & 0x3F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | (((imm >> 1) & 0xF) << 8)
        | (((imm >> 11) & 1) << 7)
        | 0x63
}
fn beq(rs1: u32, rs2: u32, imm: i32) -> u32 {
    branch(0b000, rs1, rs2, imm)
}
fn bne(rs1: u32, rs2: u32, imm: i32) -> u32 {
    branch(0b001, rs1, rs2, imm)
}
fn jal0(imm: i32) -> u32 {
    let imm = imm as u32;
    (((imm >> 20) & 1) << 31)
        | (((imm >> 1) & 0x3FF) << 21)
        | (((imm >> 11) & 1) << 20)
        | (((imm >> 12) & 0xFF) << 12)
        | 0x6f
}

const CODE: u32 = 0x4038_0000; // IRAM
const HANDLER: u32 = CODE + 0x400;
const WATCHED: u32 = 0x3FC8_1000; // DRAM
const SYSTIMER: u32 = 0x6002_3000;
const SPIN: i32 = 1000;

/// The handshake esp-hal's `systimer::Unit::value` performs: set UNIT0_OP
/// UPDATE (bit 30), poll VALUE_VALID (bit 29), read VALUE_LO into `rd`.
fn read_unit0(rd: u32) -> Vec<u32> {
    vec![
        lui(7, 0x40000), // 1 << 30
        sw(7, 0x04, 5),  // UNIT0_OP <- UPDATE
        lui(7, 0x20000), // 1 << 29
        lw(9, 0x04, 5),  // poll: UNIT0_OP
        and(9, 9, 7),
        beq(9, 0, -8),
        lw(rd, 0x44, 5), // UNIT0_VALUE_LO
    ]
}

#[test]
fn hand_assembled_watch_and_systimer_handshake_on_the_c3_yaml_bus() {
    // The bus exactly as a bare-ELF fast boot builds it from the chip YAML.
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();

    let mut prog = vec![
        lui(5, SYSTIMER >> 12),
        lui(6, WATCHED >> 12),
        // Trap vector.
        lui(7, HANDLER >> 12),
        addi(7, 7, (HANDLER & 0xFFF) as i32),
        csrrw(0, 0x305, 7), // mtvec
        // Trigger 0: m | store | NAPOT over the 4-byte word at WATCHED, enabled.
        csrrw(0, 0x7A0, 0),
        addi(7, 0, 1 << 3),
        csrrw(0, 0x7A5, 7), // tcontrol.mte
        addi(7, 0, (1 << 6) | (1 << 1) | (1 << 7)),
        csrrw(0, 0x7A1, 7), // tdata1
        addi(7, 6, 1),
        csrrw(0, 0x7A2, 7), // tdata2 = WATCHED | 0b01
        // A store next to the guard must not trap.
        sw(0, 4, 6),
    ];
    prog.extend(read_unit0(18)); // s2 = first sample
    prog.extend([addi(8, 0, SPIN), addi(8, 8, -1), bne(8, 0, -4)]);
    prog.extend(read_unit0(19)); // s3 = second sample
    prog.extend([addi(10, 0, 0x55), sw(10, 0, 6), addi(11, 0, 1), jal0(0)]);
    for (i, w) in prog.iter().enumerate() {
        bus.write_u32(u64::from(CODE) + 4 * i as u64, *w).unwrap();
    }
    // Handler: capture mcause / mepc / mtval / tdata1, then park.
    let handler = [
        csrrs(20, 0x342, 0),
        csrrs(21, 0x341, 0),
        csrrs(22, 0x343, 0),
        csrrs(23, 0x7A1, 0),
        jal0(0),
    ];
    for (i, w) in handler.iter().enumerate() {
        bus.write_u32(u64::from(HANDLER) + 4 * i as u64, *w)
            .unwrap();
    }

    let mut cpu = RiscV::new();
    cpu.pc = CODE;
    let mut m = Machine::new(cpu, bus);
    m.run(Some(20_000)).unwrap();

    let x = |r: usize| m.cpu.x[r];
    // SYSTIMER: the spin is 2 instructions x SPIN, one cycle each, plus the
    // handshake around it; at 10 CPU cycles per 16 MHz tick that is ~200 ticks.
    let ticks = x(19).wrapping_sub(x(18));
    assert!(
        (195..=215).contains(&ticks),
        "UNIT0 advanced {ticks} ticks over ~{} cycles",
        2 * SPIN
    );
    // Trigger: the store to WATCHED trapped before retiring.
    assert_eq!(x(20), 3, "mcause = breakpoint");
    let store_pc = CODE + 4 * (prog.len() as u32 - 3);
    assert_eq!(x(21), store_pc, "mepc = the store");
    assert_eq!(x(22), WATCHED, "mtval = the watched address");
    assert_ne!(x(23) & (1 << 20), 0, "tdata1.hit");
    assert_eq!(x(11), 0, "nothing after the store ran");
    assert_eq!(m.bus.read_u32(u64::from(WATCHED)).unwrap(), 0);
}
