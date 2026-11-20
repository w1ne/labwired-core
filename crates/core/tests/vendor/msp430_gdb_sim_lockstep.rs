// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The MSP430 core against the GDB `msp430-elf` simulator, register for
//! register after every instruction, and the onboarding smoke on each
//! runnable MSP430 part.
//!
//! The golden traces in `tests/fixtures/msp430/*.trace.z` were recorded with
//! `msp430-elf-gdb` 9.1 (`target sim`) from TI msp430-gcc 9.3.1.11; see
//! `tests/fixtures/msp430/build.sh`. Two programs, on the MSP430F149 (16-bit
//! CPU) and the MSP430F2619 (CPUX, small and large memory model):
//!
//! * `torture.c`: compiler-generated code (integer arithmetic at every width,
//!   the hardware multiplier, libgcc division and shifts, recursion, indirect
//!   calls, a jump table, block moves).
//! * `isa.S`: every instruction and addressing mode over a table of operand
//!   values, including the status bits after each one.
//!
//! The simulator is a second implementation, not silicon, and it departs
//! from the user's guides (SLAU049, SLAU144) in places. The comparison
//! follows the guides there:
//!
//! * The simulator keeps the overflow flag in SR bit 7 (`MSP430_FLAG_V` is
//!   0x80 in its decoder); V is SR bit 8. The golden SR is read with bit 7
//!   moved to bit 8.
//! * Status bits it computes differently (`simulator_departs`): V after
//!   `SUB`/`SUBA` (never set), V after `RRC`/`RRA` (set with the result's top bit),
//!   V after `DADD` (undefined in the guides), C after `SXT` (an unset local). Those bits are not compared; the core
//!   takes the simulator's value afterwards so the two runs stay in step, and
//!   unit tests in `cpu/msp430_tests.rs` hold the guides' values.
//! * `SXT Rdst` on the CPUX: the guide extends the sign into bits 19:8, the
//!   simulator stores 16 bits. `SWPBX.A Rdst`: the guide leaves bits 19:16
//!   alone, the simulator clears them. The core's upper bits are checked
//!   against the guide and then set to the simulator's.
//! * Operand forms it resolves differently (a symbolic `CALL` operand, a
//!   negative `CALLA` index, `DADDX.A`) are not in the fixtures; unit tests
//!   in `cpu/msp430_tests.rs` cover them.

use super::common;
use labwired_core::bus::SystemBus;
use labwired_core::{Bus, Cpu};

fn golden(name: &str) -> Vec<[u32; 16]> {
    let packed = common::committed(&format!("tests/fixtures/msp430/{name}.trace.z"));
    let text = miniz_oxide::inflate::decompress_to_vec_zlib(&packed).expect("zlib trace");
    let text = String::from_utf8(text).expect("utf-8 trace");
    let mut lines = text.lines();
    let mut regs = [0u32; 16];
    for (slot, word) in regs.iter_mut().zip(lines.next().unwrap().split(' ')) {
        *slot = u32::from_str_radix(word, 16).unwrap();
    }
    let mut rows = vec![regs];
    for line in lines {
        for change in line.split(' ').filter(|c| !c.is_empty()) {
            let (index, value) = change.split_once('=').unwrap();
            regs[usize::from_str_radix(index, 16).unwrap()] =
                u32::from_str_radix(value, 16).unwrap();
        }
        rows.push(regs);
    }
    rows
}

fn lockstep(system: &str, name: &str, at_least: usize) {
    let (chip, manifest) = common::system(system);
    let elf = common::committed(&format!("tests/fixtures/msp430/{name}.elf"));
    let bus = SystemBus::from_config_with_plugins(&chip, &manifest, &[]).unwrap();
    let mut machine = labwired_core::system::msp430::load_elf(bus, &chip, &elf).unwrap();
    let rows = golden(name);
    assert!(
        rows.len() > at_least,
        "{name}: the golden trace holds only {} instructions",
        rows.len() - 1
    );
    let registers =
        |cpu: &dyn Cpu| -> [u32; 16] { std::array::from_fn(|i| cpu.get_register(i as u8)) };
    // The simulator starts with SP = 0 at the ELF entry, as the core does at
    // the reset vector.
    assert_eq!(
        registers(machine.cpu.as_ref()),
        rows[0],
        "{name}: reset state"
    );
    let cpux =
        machine.cpu.get_register_names().len() == 16 && chip.core.as_deref() == Some("msp430x");
    for (n, row) in rows.iter().enumerate().skip(1) {
        let pc = machine.cpu.get_pc();
        let mut opcode = machine.bus.read_u16(u64::from(pc)).unwrap_or(0);
        let extended = cpux && opcode & 0xF800 == 0x1800;
        let ext_word = opcode;
        if extended {
            opcode = machine.bus.read_u16(u64::from(pc) + 2).unwrap_or(0);
        }
        machine
            .step()
            .unwrap_or_else(|e| panic!("{name}: instruction {n} at {pc:#x}: {e}"));
        let mut got = registers(machine.cpu.as_ref());
        let mut want = *row;
        // The simulator's V is SR bit 7.
        want[2] = (want[2] & !0x180) | (want[2] & 0x80) << 1;
        // Where the simulator departs from the guides the core keeps the
        // guide's value; adopt the simulator's bits so the runs stay in step.
        let address_word = extended && ext_word & 0x40 == 0;
        if cpux && opcode & 0xFFF0 == 0x1180 && !address_word {
            // SXT Rdst on the CPUX fills bits 19:8 (SLAU144); the simulator
            // stores 16 bits.
            let r = usize::from(opcode & 0xF);
            let sign = if got[r] & 0x80 != 0 { 0xFFF00 } else { 0 };
            assert_eq!(got[r] & 0xFFF00, sign, "{name}: SXT at {pc:#x}");
            got[r] &= 0xFFFF;
            machine.cpu.set_register(r as u8, got[r]);
        }
        if address_word && opcode & 0xFFF0 == 0x1080 {
            // SWPBX.A Rdst leaves bits 19:16 alone (SLAU144; only SWPBX.W
            // clears them); the simulator clears them.
            let r = usize::from(opcode & 0xF);
            assert_eq!(
                got[r] & 0xF0000,
                rows[n - 1][r] & 0xF0000,
                "{name}: SWPBX.A at {pc:#x}"
            );
            got[r] &= 0xFFFF;
            machine.cpu.set_register(r as u8, got[r]);
        }
        let departs = simulator_departs(opcode, cpux && !extended);
        if departs != 0 {
            got[2] = (got[2] & !departs) | (want[2] & departs);
            machine.cpu.set_register(2, got[2]);
        }
        assert!(
            got == want,
            "{name}: instruction {n} at {pc:#x} (opcode {opcode:#06x}) diverged\n  \
             before {:x?}\n  got    {got:x?}\n  want   {want:x?}",
            rows[n - 1],
        );
    }
}

/// SR bits the simulator computes against the user's guides for `opcode`.
fn simulator_departs(opcode: u16, cpux: bool) -> u32 {
    const C: u32 = 1;
    const V: u32 = 0x100;
    match opcode {
        // SUB, SUBA: the simulator never sets V (CMP and SUBC do).
        w if w >> 12 == 8 => V,
        w if cpux && w >> 12 == 0 && matches!(w >> 4 & 0xF, 0xB | 0xF) => V,
        // DADD: V is undefined in the guides; the core leaves it unchanged.
        w if w >> 12 == 10 => V,
        // RRC, RRA: the simulator sets V with the result's top bit; the
        // guides reset it.
        w if w & 0xFF80 == 0x1000 || w & 0xFF80 == 0x1100 => V,
        // SXT: the simulator's C is an unset local; the guides say not Z.
        w if w & 0xFF80 == 0x1180 => C,
        _ => 0,
    }
}

const F149: &str = "configs/systems/onboarding/mspsim-msp430f149.yaml";
const F2619: &str = "configs/systems/onboarding/msp430f2619.yaml";

#[test]
fn msp430_instruction_set_matches_the_gdb_simulator() {
    lockstep(F149, "isa-f149", 3_000);
}

#[test]
fn msp430_compiled_code_matches_the_gdb_simulator() {
    lockstep(F149, "torture-f149", 100_000);
}

#[test]
fn cpux_instruction_set_matches_the_gdb_simulator() {
    lockstep(F2619, "isa-f2619", 10_000);
}

#[test]
fn cpux_compiled_code_matches_the_gdb_simulator_in_the_small_model() {
    lockstep(F2619, "torture-f2619-small", 100_000);
}

#[test]
fn cpux_compiled_code_matches_the_gdb_simulator_in_the_large_model() {
    lockstep(F2619, "torture-f2619-large", 100_000);
}

// ---- the onboarding smoke through the Session builder ---------------------

/// Run `examples/renode-onboarding/msp430-uart-smoke.c` for one part: the
/// checks, a host byte through the receive interrupt, the second UART, and
/// the restart through a WDTCTL security key violation.
fn smoke(system: &str, mcu: &str, part: &str, letters: &str) {
    use labwired_core::session::{OpenOptions, Session};
    use labwired_core::system::builder::{
        BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
    };
    use std::time::Duration;

    let (chip, manifest) = common::system(system);
    let elf = common::committed(&format!("tests/fixtures/{mcu}-uart-smoke.elf"));
    let mut s = Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&elf),
            boot: BootMode::FastBoot,
            blobs: &BlobMap::new(),
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap();
    let expect = |s: &mut Session, text: &str| {
        s.expect(&regex::escape(text), Duration::from_secs(2))
            .unwrap_or_else(|e| panic!("{part}: {e}\n--- UART ---\n{}", s.uart_transcript()));
    };
    // Checks 1 to 7 need no host; check 8 waits for a byte.
    const BEFORE_HOST: &str = "PPPPZPkPzP";
    expect(&mut s, &format!("{part} checks {BEFORE_HOST}"));
    s.send(b"R");
    // The cursor sits after the first seven letters: match the rest of the line.
    let rest = letters
        .strip_prefix(BEFORE_HOST)
        .expect("letters of checks 1 to 7");
    expect(&mut s, &format!("{rest} DONE rx=52"));
    expect(&mut s, &format!("{part} PUC WDTIFG=1"));
    expect(&mut s, &format!("{part} smoke complete"));
    let out = s.uart_transcript();
    assert!(out.contains(&format!("{part} UART1 ok")), "{out}");
    assert!(!out.contains("PUC missing"), "{out}");
}

#[test]
fn msp430f149_smoke() {
    smoke(F149, "msp430f149", "MSP430F149", "PPPPZPkPzPP");
}

/// The MSP430F1611 has no memory below its flash at 0x4000, where the ELF
/// headers of the first load segment would land.
#[test]
fn msp430f1611_smoke() {
    smoke(
        "configs/systems/onboarding/mspsim-msp430f1611.yaml",
        "msp430f1611",
        "MSP430F1611",
        "PPPPZPkPzPP",
    );
}

#[test]
fn msp430f2617_smoke() {
    smoke(
        "configs/systems/onboarding/mspsim-msp430f2617.yaml",
        "msp430f2617",
        "MSP430F2617",
        "PPPPZPkPzPPP",
    );
}

#[test]
fn msp430f2619_smoke() {
    smoke(F2619, "msp430f2619", "MSP430F2619", "PPPPZPkPzPPP");
}
