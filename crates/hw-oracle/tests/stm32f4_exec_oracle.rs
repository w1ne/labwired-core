// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32F407 **execution** oracle — behavioural depth beyond MMIO register diffs.
//!
//! Where `stm32f4_mmio_diff` drives registers and reads back values, this
//! assembles a tiny ARM Thumb-2 program, **runs it on the modeled F407 bus AND
//! on real F407 silicon**, and compares the resulting state. That exercises the
//! parts a register sweep cannot reach: the NVIC, exception entry/exit, ISR
//! execution, and the DWT cycle-counter mechanism.
//!
//! Each `#[thumb_oracle_test]` expands into a sim-instruction test, a
//! sim-peripheral (full-chip bus) test, and an `--ignored` hardware test.
//!
//! ```text
//! # sim (CI):
//! cargo test -p labwired-hw-oracle --test stm32f4_exec_oracle
//! # hardware (F407 on its ST-Link; clone dongles share garbage serials → pin
//! # the probe by USB location):
//! STM32_TARGET=stm32f4x LABWIRED_STLINK_LOCATION=1-1 \
//!   cargo test -p labwired-hw-oracle --test stm32f4_exec_oracle \
//!     --features hw-oracle-stm32 -- --ignored --test-threads=1
//! ```
//! `--test-threads=1` is required on hardware: the `_hw` + `_diff` variants
//! share the single ST-Link, so they must run serially.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::arm_thumb::{
    assemble, bx, cmp_reg, cpsie_i, it, ldr_imm5, movs_imm8, movt_imm16, movw_imm16, str_imm5,
    Thumb, ThumbOracleCase, INIT_SP, PROG_BASE_HW,
};
use labwired_hw_oracle::thumb_oracle_test;
use std::path::PathBuf;

// ── F407 register map (RM0090) ───────────────────────────────────────────────
// EXTI is at 0x4001_3C00 on F4 (vs 0x4001_0400 on F1). NVIC/SCB/DWT are
// core-level (same addresses on every Cortex-M).
const EXTI_BASE: u32 = 0x4001_3C00;
const EXTI_IMR: u32 = EXTI_BASE;
const EXTI_SWIER: u32 = EXTI_BASE + 0x10;
const EXTI_PR: u32 = EXTI_BASE + 0x14;

/// Build the full-chip F407 sim bus. `run_capture` wires the Cortex-M system
/// block (NVIC @0xE000E100, SCB @0xE000ED00, DWT @0xE000_1000) when it builds
/// the CPU, so interrupt-delivery oracles get a CPU sharing the bus's NVIC/VTOR.
fn f407_bus() -> SystemBus {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = dir.join("../../configs/chips/stm32f407.yaml");
    let system_path = dir.join("../../configs/systems/nucleo-f407.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();
    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build F407 sim bus: {e}"))
}

fn load_addr(rd: u8, addr: u32) -> [Thumb; 2] {
    [
        Thumb::W(movw_imm16(rd, (addr & 0xFFFF) as u16)),
        Thumb::W(movt_imm16(rd, (addr >> 16) as u16)),
    ]
}

fn store_imm32(value: u32) -> [Thumb; 3] {
    [
        Thumb::W(movw_imm16(1, (value & 0xFFFF) as u16)),
        Thumb::W(movt_imm16(1, (value >> 16) as u16)),
        Thumb::H(str_imm5(1, 0, 0)),
    ]
}

/// Lay out a 128-byte vector table at the load base, with `isr` and `main`
/// appended; returns the program and the byte offset of `main` (the entry).
fn interrupt_program(isr: &[Thumb], main: &[Thumb], exc_num: usize) -> (Vec<Thumb>, u32) {
    const TABLE_ENTRIES: usize = 32; // 128 bytes — VTOR is 128-byte aligned
    let table_bytes = (TABLE_ENTRIES * 4) as u32;
    let isr_bytes = assemble(isr).len() as u32;
    let isr_addr = PROG_BASE_HW + table_bytes;
    let main_offset = table_bytes + isr_bytes;

    let mut prog: Vec<Thumb> = vec![Thumb::Data(0); TABLE_ENTRIES];
    prog[0] = Thumb::Data(INIT_SP);
    prog[1] = Thumb::Data((PROG_BASE_HW + main_offset) | 1);
    prog[exc_num] = Thumb::Data(isr_addr | 1);
    prog.extend_from_slice(isr);
    prog.extend_from_slice(main);
    (prog, main_offset)
}

/// **The deepest behavioural check.** Configure EXTI line 0, software-trigger it
/// (SWIER), and prove the interrupt is *delivered*: the NVIC pends IRQ6, the
/// core takes exception 22 (stacking + vector fetch via VTOR), the ISR executes
/// (writes a marker and clears the pending bit), and BX LR returns cleanly.
/// Validates the whole NVIC + exception path on real F407 silicon — impossible
/// to reach from a register sweep.
#[thumb_oracle_test]
fn exti0_interrupt_delivery() -> ThumbOracleCase {
    const VTOR_REG: u32 = 0xE000_ED08; // SCB->VTOR
    const NVIC_ISER0: u32 = 0xE000_E100;
    const MARKER: u32 = 0x2000_0300;
    const MARKER_VALUE: u32 = 0xABCD_1234;
    const EXTI0_EXC: usize = 16 + 6; // IRQ6 → exception 22

    // ISR: write marker, clear EXTI pending line 0, exception-return.
    let mut isr: Vec<Thumb> = Vec::new();
    isr.extend(load_addr(0, MARKER));
    isr.extend(store_imm32(MARKER_VALUE));
    isr.extend(load_addr(0, EXTI_PR));
    isr.extend(store_imm32(0x1)); // rc_w1: clear line 0
    isr.push(Thumb::H(bx(14))); // BX LR

    // main: relocate VTOR, clear marker, unmask + enable + trigger IRQ.
    let mut main: Vec<Thumb> = Vec::new();
    main.extend(load_addr(0, VTOR_REG));
    main.extend(store_imm32(PROG_BASE_HW));
    main.extend(load_addr(0, MARKER));
    main.extend(store_imm32(0x0));
    main.extend(load_addr(0, EXTI_IMR));
    main.extend(store_imm32(0x1)); // unmask line 0
    main.extend(load_addr(0, NVIC_ISER0));
    main.extend(store_imm32(1 << 6)); // enable IRQ6
    main.push(Thumb::H(cpsie_i())); // clear PRIMASK
    main.extend(load_addr(0, EXTI_SWIER));
    main.extend(store_imm32(0x1)); // software-trigger line 0

    let (prog, entry) = interrupt_program(&isr, &main, EXTI0_EXC);

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f407_bus)
        .entry_offset(entry)
        .live_peripherals(true)
        .capture_mem(&[MARKER, EXTI_PR])
        .expect(|st| {
            st.assert_mem(MARKER, MARKER_VALUE); // the ISR ran (exception delivered)
            st.assert_mem(EXTI_PR, 0); // the ISR cleared the pending bit
        })
}

/// DWT cycle counter mechanism: enable DEMCR.TRCENA + DWT_CTRL.CYCCNTENA, reset
/// CYCCNT, then read it back — it has advanced (cycles elapsed). A self-relative
/// boolean (CYCCNT != 0), because the absolute count diverges (the sim is
/// instruction-level, not cycle-accurate); the *mechanism* (enable→count→read)
/// is invariant and pinned on both sim and silicon.
#[thumb_oracle_test]
fn dwt_cyccnt_advances() -> ThumbOracleCase {
    const DEMCR: u32 = 0xE000_EDFC;
    const DEMCR_TRCENA: u32 = 1 << 24;
    const DWT_CTRL: u32 = 0xE000_1000;
    const DWT_CYCCNT: u32 = 0xE000_1004;
    const DWT_CTRL_CYCCNTENA: u32 = 1 << 0;
    const MARKER: u32 = 0x2000_0304;

    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(load_addr(0, DEMCR));
    prog.extend(store_imm32(DEMCR_TRCENA));
    prog.extend(load_addr(0, DWT_CTRL));
    prog.extend(store_imm32(DWT_CTRL_CYCCNTENA));
    prog.extend(load_addr(0, DWT_CYCCNT));
    prog.extend(store_imm32(0)); // reset
    prog.extend(load_addr(0, DWT_CYCCNT));
    prog.push(Thumb::H(ldr_imm5(3, 0, 0))); // r3 = CYCCNT (nonzero — cycles elapsed)
                                            // marker = (r3 != 0) ? 1 : 0, via an IT NE block (no branches).
    prog.push(Thumb::H(movs_imm8(1, 0))); // r1 = 0
    prog.push(Thumb::H(movs_imm8(2, 0))); // r2 = 0
    prog.push(Thumb::H(cmp_reg(3, 2))); // Z = (CYCCNT == 0)
    prog.push(Thumb::H(it(0b0001, 0x8))); // IT NE
    prog.push(Thumb::H(movs_imm8(1, 1))); // r1 = 1 iff CYCCNT != 0
    prog.push(Thumb::H(movs_imm8(3, 0))); // clear the divergent count → deterministic final state
    prog.extend(load_addr(0, MARKER));
    prog.push(Thumb::H(str_imm5(1, 0, 0))); // marker = r1

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f407_bus)
        .live_peripherals(true) // tick the DWT each step so CYCCNT advances in sim
        .capture_mem(&[MARKER])
        .expect(|st| st.assert_mem(MARKER, 1)) // counter advanced on both sim and silicon
}
