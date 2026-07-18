// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2040 DMA **peripheral-execution** oracle bank (datasheet §2.5).
//!
//! Closes the executing-fidelity loop for the DMA controller: it executes real
//! Thumb machine code on the **full RP2040 chip bus** (`SystemBus::from_config`,
//! the same path the runtime builds), programs the DMA through its MMIO alias
//! registers, and — with the peripheral walk live — asserts the byte-exact
//! dynamics a register poke can't reach:
//!
//! 1. `dma_m2m_copies_and_increments` — a memory-to-memory transfer moves the
//!    exact source bytes to the destination, `READ_ADDR`/`WRITE_ADDR` advance,
//!    and `TRANS_COUNT` drains to 0.
//! 2. `dma_chain_hands_off_to_second_channel` — channel 0 completes and its
//!    `CHAIN_TO` triggers a pre-armed channel 1, which then moves its own
//!    buffer, with no CPU intervention between the two.
//! 3. `dma_completion_delivers_irq` — completion latches `INTR`, the
//!    `INTS0` aggregator asserts `DMA_IRQ_0` (NVIC 11), the CPU vectors to the
//!    ISR, which acknowledges via `INTS0` write-1-clear.
//!
//! The RP2040 has no attached bench target, so only the always-compiled `_sim`
//! variant runs. The engine paces one beat per executed instruction — the
//! absolute rate is arbitrary (the sim has no wall clock) but strictly
//! deterministic, so byte contents and final counts are exact regression locks.
//!
//! ```text
//! cargo test -p labwired-hw-oracle --test rp2040_dma_oracle
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::arm_thumb::{
    assemble, bx, movt_imm16, movw_imm16, str_imm5, Thumb, ThumbOracleCase, INIT_SP, PROG_BASE_HW,
};
use labwired_hw_oracle::thumb_oracle_test;
use std::path::PathBuf;

// ── RP2040 DMA register map (§2.5.7, base 0x50000000) ───────────────────────
const DMA_BASE: u32 = 0x5000_0000;
const CH_STRIDE: u32 = 0x40;
const READ_ADDR: u32 = 0x00;
const WRITE_ADDR: u32 = 0x04;
const TRANS_COUNT: u32 = 0x08;
const CTRL_TRIG: u32 = 0x0c; // alias-0 trigger (writes CTRL, then starts)
const AL1_CTRL: u32 = 0x10; // CTRL, no trigger
const INTE0: u32 = 0x404; // IRQ0 enable
const INTS0: u32 = 0x40c; // (INTR|INTF0)&INTE0; W1C acks

fn ch(n: u32) -> u32 {
    DMA_BASE + n * CH_STRIDE
}

// ── CTRL_TRIG field encodings (§2.5.7) ──────────────────────────────────────
const CTRL_EN: u32 = 1 << 0;
const CTRL_INCR_READ: u32 = 1 << 4;
const CTRL_INCR_WRITE: u32 = 1 << 5;
const CTRL_TREQ_PERMANENT: u32 = 0x3F << 15;
fn chain_to(n: u32) -> u32 {
    n << 11
}

/// Byte-size, increment both addresses, permanent (M2M) TREQ, chaining to `c`.
/// `chain_to == own index` means "no chain" per the datasheet.
fn m2m_ctrl(chain: u32) -> u32 {
    CTRL_EN | CTRL_INCR_READ | CTRL_INCR_WRITE | CTRL_TREQ_PERMANENT | chain_to(chain)
}

// RAM scratch, clear of the program (table @ PROG_BASE_HW=0x2000_2000, growing
// up) and the stack (down from INIT_SP=0x2000_4FF8).
const SRC0: u32 = 0x2000_0300;
const DST0: u32 = 0x2000_0400;
const SRC1: u32 = 0x2000_0500;
const DST1: u32 = 0x2000_0600;
const MARKER: u32 = 0x2000_0700;
const MARKER_VALUE: u32 = 0x600D_D1A1;
/// Harmless scratch slot the filler writes to — kept clear of every captured
/// address (buffers, DMA registers, MARKER) so filler never perturbs a result.
const FILLER_SLOT: u32 = 0x2000_0800;

const WORD0: u32 = 0xDEAD_BEEF;
const WORD1: u32 = 0x1234_5678;

// Cortex-M0+ system registers (SCS).
const VTOR_REG: u32 = 0xE000_ED08;
const NVIC_ISER0: u32 = 0xE000_E100;

/// `MOV.W rd,#lo ; MOVT rd,#hi` — materialise a 32-bit address/const in `rd`.
fn load_addr(rd: u8, addr: u32) -> [Thumb; 2] {
    [
        Thumb::W(movw_imm16(rd, (addr & 0xFFFF) as u16)),
        Thumb::W(movt_imm16(rd, (addr >> 16) as u16)),
    ]
}

/// `MOV.W r1,#lo ; MOVT r1,#hi ; STR r1,[r0]` — store a 32-bit immediate to the
/// address already in r0.
fn store_imm32(value: u32) -> [Thumb; 3] {
    [
        Thumb::W(movw_imm16(1, (value & 0xFFFF) as u16)),
        Thumb::W(movt_imm16(1, (value >> 16) as u16)),
        Thumb::H(str_imm5(1, 0, 0)),
    ]
}

/// Materialise `addr` in r0 and store `value` there.
fn write32(addr: u32, value: u32) -> Vec<Thumb> {
    let mut s = Vec::new();
    s.extend(load_addr(0, addr));
    s.extend(store_imm32(value));
    s
}

/// Program one DMA channel's four core registers (no trigger yet); returns the
/// instruction stream. `ctrl_off` selects the CTRL variant (trigger or not).
fn program_channel(
    base: u32,
    src: u32,
    dst: u32,
    count: u32,
    ctrl_off: u32,
    ctrl_val: u32,
) -> Vec<Thumb> {
    let mut s = Vec::new();
    s.extend(write32(base + READ_ADDR, src));
    s.extend(write32(base + WRITE_ADDR, dst));
    s.extend(write32(base + TRANS_COUNT, count));
    s.extend(write32(base + ctrl_off, ctrl_val)); // last: CTRL_TRIG starts it
    s
}

/// Distinct-PC filler so the peripheral walk advances the DMA `n` beats while
/// the CPU is still executing (one beat per executed instruction). Each store
/// to the (harmless) marker slot is one instruction → one tick.
fn filler(n: usize) -> Vec<Thumb> {
    let mut s = Vec::new();
    for _ in 0..n {
        s.extend(load_addr(0, FILLER_SLOT));
        s.push(Thumb::H(str_imm5(0, 0, 0))); // *FILLER_SLOT = FILLER_SLOT (harmless)
    }
    s
}

/// Build the full RP2040 simulator bus, matching `SystemBus::from_config`.
fn rp2040_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/rp2040.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "rp2040-dma-exec-oracle".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build RP2040 sim bus: {e}"))
}

// ── 1. Memory-to-memory copy + address increment + TRANS_COUNT drain ─────────
//
// Seed two source words, program channel 0 for an 8-byte byte-wide M2M
// transfer with both addresses incrementing, trigger via CTRL_TRIG, then spin
// filler so the live walk moves every beat. The destination must hold the
// exact source bytes, READ_ADDR/WRITE_ADDR must have advanced by 8, and
// TRANS_COUNT must read 0 — none reachable by a poke.
#[thumb_oracle_test]
fn dma_m2m_copies_and_increments() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(write32(SRC0, WORD0));
    prog.extend(write32(SRC0 + 4, WORD1));
    prog.extend(program_channel(
        ch(0),
        SRC0,
        DST0,
        8,
        CTRL_TRIG,
        m2m_ctrl(0),
    ));
    prog.extend(filler(24)); // > 8 beats so the transfer completes mid-run

    ThumbOracleCase::mixed(&prog)
        .sim_bus(rp2040_bus)
        .live_peripherals(true)
        .capture_mem(&[
            DST0,
            DST0 + 4,
            ch(0) + TRANS_COUNT,
            ch(0) + READ_ADDR,
            ch(0) + WRITE_ADDR,
        ])
        .expect(|st| {
            st.assert_mem(DST0, WORD0);
            st.assert_mem(DST0 + 4, WORD1);
            st.assert_mem(ch(0) + TRANS_COUNT, 0);
            st.assert_mem(ch(0) + READ_ADDR, SRC0 + 8);
            st.assert_mem(ch(0) + WRITE_ADDR, DST0 + 8);
        })
}

// ── 2. Two-channel CHAIN_TO handoff ──────────────────────────────────────────
//
// Pre-arm channel 1 (enabled, permanent TREQ, self-chain = no further chain)
// via AL1_CTRL (no trigger), then trigger channel 0 with CHAIN_TO=1. Channel 0
// drains, its completion triggers channel 1, and channel 1 moves its own
// buffer — all with no CPU writes in between. Both destinations must match.
#[thumb_oracle_test]
fn dma_chain_hands_off_to_second_channel() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(write32(SRC0, WORD0));
    prog.extend(write32(SRC1, WORD1));
    // Channel 1 armed but idle (AL1_CTRL does not trigger). Self-chain (1).
    prog.extend(program_channel(ch(1), SRC1, DST1, 4, AL1_CTRL, m2m_ctrl(1)));
    // Channel 0 triggers now and chains to channel 1 on completion.
    prog.extend(program_channel(
        ch(0),
        SRC0,
        DST0,
        4,
        CTRL_TRIG,
        m2m_ctrl(1),
    ));
    prog.extend(filler(32)); // 4 + 4 beats + margin

    ThumbOracleCase::mixed(&prog)
        .sim_bus(rp2040_bus)
        .live_peripherals(true)
        .capture_mem(&[DST0, DST1, ch(0) + TRANS_COUNT, ch(1) + TRANS_COUNT])
        .expect(|st| {
            st.assert_mem(DST0, WORD0); // channel 0 moved its word
            st.assert_mem(DST1, WORD1); // channel 1 ran via CHAIN_TO
            st.assert_mem(ch(0) + TRANS_COUNT, 0);
            st.assert_mem(ch(1) + TRANS_COUNT, 0);
        })
}

// ── 3. Completion IRQ through NVIC ───────────────────────────────────────────
//
// Enable INTE0 for channel 0 and unmask NVIC IRQ 11 (DMA_IRQ_0). A one-beat
// M2M transfer completes, latches INTR bit 0; INTS0 asserts DMA_IRQ_0
// level-sensitively; the CPU vectors to the ISR (exception 27 = IRQ 11 + 16),
// which writes a RAM marker and acknowledges via INTS0 write-1-clear. Final
// state: marker set (exception delivered), INTS0 clear (ISR acked).
#[thumb_oracle_test]
fn dma_completion_delivers_irq() -> ThumbOracleCase {
    const DMA_IRQ0_EXC: usize = 16 + 11; // NVIC IRQ 11 → exception number 27

    // ISR: write the marker, acknowledge INTS0 bit 0 (W1C), return.
    let mut isr: Vec<Thumb> = Vec::new();
    isr.extend(write32(MARKER, MARKER_VALUE));
    isr.extend(write32(DMA_BASE + INTS0, 1)); // ack channel-0 interrupt
    isr.push(Thumb::H(bx(14))); // BX LR — exception return

    // main: relocate VTOR, zero the marker, enable INTE0 + NVIC IRQ 11, seed a
    // source word, program a one-beat channel-0 transfer, trigger, then spin.
    let mut main: Vec<Thumb> = Vec::new();
    main.extend(write32(VTOR_REG, PROG_BASE_HW)); // VTOR = table base
    main.extend(write32(MARKER, 0)); // clear marker → proves the ISR set it
    main.extend(write32(SRC0, WORD0));
    main.extend(write32(DMA_BASE + INTE0, 1)); // enable channel-0 IRQ on line 0
    main.extend(write32(NVIC_ISER0, 1 << 11)); // unmask NVIC IRQ 11 (DMA_IRQ_0)
    main.extend(program_channel(
        ch(0),
        SRC0,
        DST0,
        4,
        CTRL_TRIG,
        m2m_ctrl(0),
    ));
    main.extend(filler(24)); // let the transfer complete + IRQ vector mid-run

    let (prog, entry) = interrupt_program(&isr, &main, DMA_IRQ0_EXC);

    ThumbOracleCase::mixed(&prog)
        .sim_bus(rp2040_bus)
        .entry_offset(entry)
        .live_peripherals(true)
        .capture_mem(&[MARKER, DMA_BASE + INTS0, DST0])
        .expect(|st| {
            st.assert_mem(DST0, WORD0); // the transfer actually moved bytes
            st.assert_mem(MARKER, MARKER_VALUE); // ISR ran → exception delivered
            st.assert_mem(DMA_BASE + INTS0, 0); // ISR acknowledged the interrupt
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
    prog[1] = Thumb::Data((PROG_BASE_HW + main_offset) | 1); // reset vector (unused)
    prog[exc_num] = Thumb::Data(isr_addr | 1); // the handler under test
    prog.extend_from_slice(isr);
    prog.extend_from_slice(main);
    (prog, main_offset)
}
