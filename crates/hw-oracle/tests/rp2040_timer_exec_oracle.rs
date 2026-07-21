// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2040 timer **peripheral-execution** oracle bank.
//!
//! Where `rp2040_reset_conformance` pins the static reset estate, this bank
//! closes the executing-fidelity loop for the free-running microsecond timer
//! (RP2040 Datasheet §4.6): it executes real Thumb machine code on the **full
//! RP2040 chip bus**, driving the timer through its MMIO interface with the
//! peripheral walk live, and asserts the dynamics a register poke can't reach —
//! the counter's monotonic advance, `PAUSE` freeze/resume, and the alarm →
//! NVIC → ISR interrupt-delivery path end to end.
//!
//! The RP2040 has no attached bench target, so only the always-compiled `_sim`
//! variant runs (the macro's `_hw` / `_diff` variants are feature-gated and
//! `#[ignore]`). Sim advances the counter one tick per executed instruction —
//! the absolute rate is arbitrary (the sim has no wall clock) but strictly
//! deterministic, so the pinned deltas below are exact regression locks, not
//! timing-fidelity claims.
//!
//! ```text
//! cargo test -p labwired-hw-oracle --test rp2040_timer_exec_oracle
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::arm_thumb::{
    adds_imm8, assemble, bx, ldr_imm5, movs_imm8, movt_imm16, movw_imm16, str_imm5, Thumb,
    ThumbOracleCase, INIT_SP, PROG_BASE_HW,
};
use labwired_hw_oracle::thumb_oracle_test;
use std::path::PathBuf;

// ── RP2040 TIMER register map (§4.6, base 0x40054000) ───────────────────────────
const TIMER_BASE: u32 = 0x4005_4000;
const TIMER_ALARM0: u32 = TIMER_BASE + 0x10;
const TIMER_ARMED: u32 = TIMER_BASE + 0x20;
const TIMER_TIMERAWL: u32 = TIMER_BASE + 0x28; // live low word
const TIMER_PAUSE: u32 = TIMER_BASE + 0x30; // bit0 freezes the counter
const TIMER_INTR: u32 = TIMER_BASE + 0x34; // raw interrupt, write-1-clear
const TIMER_INTE: u32 = TIMER_BASE + 0x38; // interrupt enable

// Cortex-M0+ system registers (SCS).
const VTOR_REG: u32 = 0xE000_ED08; // SCB->VTOR
const NVIC_ISER0: u32 = 0xE000_E100;

// RAM scratch, clear of the program (table @ PROG_BASE_HW=0x2000_2000, growing
// up) and the stack (down from INIT_SP=0x2000_4FF8).
const SAMPLE_A: u32 = 0x2000_0300;
const SAMPLE_B: u32 = 0x2000_0304;
const SAMPLE_C: u32 = 0x2000_0308;
const MARKER: u32 = 0x2000_0310;
const MARKER_VALUE: u32 = 0x600D_A1A1;

/// `MOV.W rd,#lo ; MOVT rd,#hi` — materialise a 32-bit address in `rd`.
fn load_addr(rd: u8, addr: u32) -> [Thumb; 2] {
    [
        Thumb::W(movw_imm16(rd, (addr & 0xFFFF) as u16)),
        Thumb::W(movt_imm16(rd, (addr >> 16) as u16)),
    ]
}

/// `MOV.W r1,#lo ; MOVT r1,#hi ; STR r1,[r0]` — store a 32-bit immediate to the
/// MMIO/RAM address already in r0.
fn store_imm32(value: u32) -> [Thumb; 3] {
    [
        Thumb::W(movw_imm16(1, (value & 0xFFFF) as u16)),
        Thumb::W(movt_imm16(1, (value >> 16) as u16)),
        Thumb::H(str_imm5(1, 0, 0)),
    ]
}

/// `LDR r3,[addr] ; STR r3,[sample]` — sample a 32-bit MMIO register into a RAM
/// slot. Uses r0/r2 as scratch address registers.
fn sample_reg_to(src: u32, dst: u32) -> Vec<Thumb> {
    let mut s = Vec::new();
    s.extend(load_addr(0, src));
    s.push(Thumb::H(ldr_imm5(3, 0, 0))); // r3 = *src
    s.extend(load_addr(2, dst));
    s.push(Thumb::H(str_imm5(3, 2, 0))); // *dst = r3
    s
}

/// A run of distinct-PC filler instructions so the peripheral walk advances the
/// timer this many ticks while the CPU is still executing (the harness breaks
/// two steps after the PC settles on the `B .` terminator, so ticks must elapse
/// *before* then). Each `MOVS` is one instruction → one tick.
fn filler(n: usize) -> Vec<Thumb> {
    (0..n)
        .map(|i| Thumb::H(movs_imm8(4, (i & 0xFF) as u8)))
        .collect()
}

/// Build the full RP2040 simulator bus (peripherals mapped), matching the
/// runtime's `SystemBus::from_config` path.
fn rp2040_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/rp2040.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "rp2040-timer-exec-oracle".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build RP2040 sim bus: {e}"))
}

// ── 1. Free-running counter monotonic advance ───────────────────────────────────
//
// Sample TIMERAWL, run a fixed filler window, sample again. With the peripheral
// walk live (one tick per executed instruction) the second sample must be
// strictly greater — the counter genuinely advanced through CPU→bus→timer, not
// via a poke. The exact delta is the deterministic instruction count between the
// two reads and is pinned as a regression lock.
#[thumb_oracle_test]
fn rp2040_timer_free_running() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(sample_reg_to(TIMER_TIMERAWL, SAMPLE_A));
    prog.extend(filler(16));
    prog.extend(sample_reg_to(TIMER_TIMERAWL, SAMPLE_B));

    ThumbOracleCase::mixed(&prog)
        .sim_bus(rp2040_bus)
        .live_peripherals(true)
        .capture_mem(&[SAMPLE_A, SAMPLE_B])
        .expect(|st| {
            let a = st.read_mem(SAMPLE_A);
            let b = st.read_mem(SAMPLE_B);
            assert!(
                b > a,
                "TIMERAWL must advance monotonically: A=0x{a:08X} B=0x{b:08X}"
            );
            // Deterministic tick count between the two samples (filler + the
            // second sample's address-materialise/load run). Pinned from the
            // model; a change here means the timer's advance cadence or the
            // executor's per-instruction tick moved.
            assert_eq!(b - a, 22, "pinned free-running delta");
        })
}

// ── 2. PAUSE freezes the counter, clearing it resumes ───────────────────────────
//
// Set PAUSE.bit0, sample TIMERAWL twice across a filler window: frozen, so the
// two samples are equal. Then clear PAUSE and sample once more: it advances
// again. PAUSE=1 holding the counter across live ticks is the load-bearing
// assertion — a poke could never show the freeze.
#[thumb_oracle_test]
fn rp2040_timer_pause_freezes() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    // PAUSE = 1
    prog.extend(load_addr(0, TIMER_PAUSE));
    prog.extend(store_imm32(0x1));
    prog.extend(sample_reg_to(TIMER_TIMERAWL, SAMPLE_A));
    prog.extend(filler(16));
    prog.extend(sample_reg_to(TIMER_TIMERAWL, SAMPLE_B));
    // PAUSE = 0 (resume), then let it advance and sample again.
    prog.extend(load_addr(0, TIMER_PAUSE));
    prog.extend(store_imm32(0x0));
    prog.extend(filler(16));
    prog.extend(sample_reg_to(TIMER_TIMERAWL, SAMPLE_C));

    ThumbOracleCase::mixed(&prog)
        .sim_bus(rp2040_bus)
        .live_peripherals(true)
        .capture_mem(&[SAMPLE_A, SAMPLE_B, SAMPLE_C])
        .expect(|st| {
            let a = st.read_mem(SAMPLE_A);
            let b = st.read_mem(SAMPLE_B);
            let c = st.read_mem(SAMPLE_C);
            assert_eq!(a, b, "paused counter must hold: A=0x{a:08X} B=0x{b:08X}");
            assert!(
                c > b,
                "counter must resume after PAUSE clear: B=0x{b:08X} C=0x{c:08X}"
            );
        })
}

// ── 3. Alarm → NVIC → ISR interrupt delivery ────────────────────────────────────
//
// The end-to-end tick-source path: arm ALARM0 a fixed margin ahead of the live
// counter, enable INTE + the NVIC line, and let the free-running counter reach
// the target. On match the timer latches INTR, disarms, and asserts TIMER_IRQ_0
// (NVIC IRQ 0 → exception 16) level-sensitively; the CPU vectors to the ISR,
// which writes a RAM marker and acknowledges INTR (write-1-clear). Final state:
// marker set (the exception was delivered), INTR clear (ISR acked), ARMED clear
// (alarm auto-disarmed on fire). None of this is reachable by a register poke.
#[thumb_oracle_test]
fn rp2040_timer_alarm_delivers_irq() -> ThumbOracleCase {
    const TIMER_IRQ0_EXC: usize = 16; // IRQ 0 → exception number 16
    const ALARM_MARGIN: u8 = 0x40; // ticks ahead of "now" to arm the alarm

    // ISR: write the marker, acknowledge INTR (write-1-clear bit0), return.
    let mut isr: Vec<Thumb> = Vec::new();
    isr.extend(load_addr(0, MARKER));
    isr.extend(store_imm32(MARKER_VALUE));
    isr.extend(load_addr(0, TIMER_INTR));
    isr.extend(store_imm32(0x1)); // ack alarm-0 raw interrupt
    isr.push(Thumb::H(bx(14))); // BX LR — exception return

    // main: relocate VTOR, zero the marker, enable INTE + NVIC IRQ0, arm ALARM0
    // a margin ahead of the live counter, then spin through filler so the
    // counter reaches the target while the CPU is still stepping.
    let mut main: Vec<Thumb> = Vec::new();
    main.extend(load_addr(0, VTOR_REG));
    main.extend(store_imm32(PROG_BASE_HW)); // VTOR = table base
    main.extend(load_addr(0, MARKER));
    main.extend(store_imm32(0x0)); // clear marker → proves the ISR set it
    main.extend(load_addr(0, TIMER_INTE));
    main.extend(store_imm32(0x1)); // enable alarm-0 interrupt
    main.extend(load_addr(0, NVIC_ISER0));
    main.extend(store_imm32(1 << 0)); // enable NVIC IRQ0 (TIMER_IRQ_0)
                                      // ALARM0 = TIMERAWL + margin (guaranteed in the future: only a handful of
                                      // ticks elapse before the arming store, margin is comfortably larger).
    main.extend(load_addr(0, TIMER_TIMERAWL));
    main.push(Thumb::H(ldr_imm5(3, 0, 0))); // r3 = live low word
    main.push(Thumb::H(adds_imm8(3, ALARM_MARGIN))); // r3 += margin
    main.extend(load_addr(0, TIMER_ALARM0));
    main.push(Thumb::H(str_imm5(3, 0, 0))); // arm alarm 0
    main.extend(filler(128)); // > margin ticks so the match lands mid-execution

    let (prog, entry) = interrupt_program(&isr, &main, TIMER_IRQ0_EXC);

    ThumbOracleCase::mixed(&prog)
        .sim_bus(rp2040_bus)
        .entry_offset(entry)
        .live_peripherals(true)
        .capture_mem(&[MARKER, TIMER_INTR, TIMER_ARMED])
        .expect(|st| {
            st.assert_mem(MARKER, MARKER_VALUE); // ISR ran → exception delivered
            st.assert_mem(TIMER_INTR, 0); // ISR acknowledged the raw interrupt
            st.assert_mem(TIMER_ARMED, 0); // alarm auto-disarmed on fire
        })
}

/// Build `[32-entry vector table][isr][main]` and return the program plus the
/// byte offset of `main` (the entry point). `exc_num` is the exception number
/// (IRQ + 16) whose vector points at the ISR. VTOR is 128-byte aligned, so the
/// 32-entry (128-byte) table sits at the load base.
fn interrupt_program(isr: &[Thumb], main: &[Thumb], exc_num: usize) -> (Vec<Thumb>, u32) {
    const TABLE_ENTRIES: usize = 32;
    let table_bytes = (TABLE_ENTRIES * 4) as u32;
    let isr_bytes = assemble(isr).len() as u32;
    let isr_addr = PROG_BASE_HW + table_bytes;
    let main_offset = table_bytes + isr_bytes;

    let mut prog: Vec<Thumb> = vec![Thumb::Data(0); TABLE_ENTRIES];
    prog[0] = Thumb::Data(INIT_SP); // initial SP (vector 0)
    prog[1] = Thumb::Data((PROG_BASE_HW + main_offset) | 1); // reset vector (unused; PC set directly)
    prog[exc_num] = Thumb::Data(isr_addr | 1); // the handler under test
    prog.extend_from_slice(isr);
    prog.extend_from_slice(main);
    (prog, main_offset)
}
