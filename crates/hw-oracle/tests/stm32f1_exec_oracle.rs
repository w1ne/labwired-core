// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32F103 **peripheral-execution** oracle bank.
//!
//! Where `thumb_oracles` validates the CPU core (RAM-only bus) and
//! `stm32f1_mmio_diff` pokes peripheral registers directly from the test
//! harness, this bank closes the loop: it executes *real ARM machine code*
//! that drives a peripheral through its MMIO interface, on a **full chip
//! bus** in sim and on real silicon over SWD, then diffs the two.  It is the
//! end-to-end CPU→bus→peripheral integration check — the dynamics a register
//! poke can't reach (here: the TIM2 update-generation event resetting a
//! live counter and loading the ARR/PSC shadows).
//!
//! Each `#[thumb_oracle_test]` expands into three tests:
//!   * `*_sim`  — always compiled; full F103 chip bus in software.
//!   * `*_hw`   — gated on `hw-oracle-stm32`, `#[ignore]`; SWD-attached F103.
//!   * `*_diff` — gated on `hw-oracle-stm32`, `#[ignore]`; runs both + diffs.
//!
//! Sim only:
//! ```text
//! cargo test -p labwired-hw-oracle --test stm32f1_exec_oracle
//! ```
//! HW / diff (Blue Pill on ST-Link, OpenOCD installed):
//! ```text
//! STM32_TARGET=stm32f1x cargo test -p labwired-hw-oracle --test stm32f1_exec_oracle \
//!     --features hw-oracle-stm32 -- --ignored --test-threads=1
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::arm_thumb::{
    ldr_imm5, movs_imm8, movt_imm16, movw_imm16, orrs, str_imm5, Thumb, ThumbOracleCase,
};
use labwired_hw_oracle::thumb_oracle_test;
use std::path::PathBuf;

// ── F103 register map (RM0008) ─────────────────────────────────────────────────

/// RCC APB1 peripheral-clock enable register (RCC base 0x4002_1000 + 0x1C).
const RCC_APB1ENR: u32 = 0x4002_101C;
/// TIM2EN bit in RCC_APB1ENR.
const RCC_APB1ENR_TIM2EN: u32 = 1 << 0;

/// RCC APB2 clock-enable (RCC + 0x18) and the GPIOA enable bit (IOPAEN).
const RCC_APB2ENR: u32 = 0x4002_1018;
const RCC_APB2ENR_IOPAEN: u32 = 1 << 2;
/// RCC AHB clock-enable (RCC + 0x14) and the CRC enable bit (CRCEN).
const RCC_AHBENR: u32 = 0x4002_1014;
const RCC_AHBENR_CRCEN: u32 = 1 << 6;

/// GPIOA (RM0008): output-data + atomic set/reset registers.
const GPIOA_BASE: u32 = 0x4001_0800;
const GPIOA_ODR: u32 = GPIOA_BASE + 0x0C; // output data
const GPIOA_BSRR: u32 = GPIOA_BASE + 0x10; // atomic set (lo16) / reset (hi16)
const GPIOA_BRR: u32 = GPIOA_BASE + 0x14; // atomic reset (lo16)

/// CRC unit (RM0008): data register + control.
const CRC_BASE: u32 = 0x4002_3000;
const CRC_DR: u32 = CRC_BASE + 0x00; // data in / CRC result out
const CRC_CR: u32 = CRC_BASE + 0x08; // control (RESET = bit 0)

const TIM2_BASE: u32 = 0x4000_0000;
const TIM2_SR: u32 = TIM2_BASE + 0x10; // status (UIF=bit0, CC1..4IF=bits1..4)

/// TIM2_SR after a bare UG event from the reset register state, **observed on
/// STM32F103 silicon**: UIF (bit 0) plus all four compare-match flags
/// CC1IF..CC4IF (bits 1..4). The UG reload sets CNT=0, which equals every
/// CCRx (all reset to 0) with the channels in output-compare mode (CCMR reset)
/// — so each channel latches a compare match. Documented STM32 gotcha; this
/// oracle pins it.
const TIM2_SR_AFTER_UG: u32 = 0x1F;
const TIM2_EGR: u32 = TIM2_BASE + 0x14; // event generation (UG = bit 0)
const TIM2_CNT: u32 = TIM2_BASE + 0x24; // counter
const TIM2_PSC: u32 = TIM2_BASE + 0x28; // prescaler
const TIM2_ARR: u32 = TIM2_BASE + 0x2C; // auto-reload

/// Emit `MOV.W rd,#lo ; MOVT rd,#hi` to materialise the 32-bit `addr` in `rd`
/// (no literal pool needed).
fn load_addr(rd: u8, addr: u32) -> [Thumb; 2] {
    [
        Thumb::W(movw_imm16(rd, (addr & 0xFFFF) as u16)),
        Thumb::W(movt_imm16(rd, (addr >> 16) as u16)),
    ]
}

/// `MOV.W r1,#imm ; STR r1,[r0]` — store a 16-bit immediate to the MMIO
/// address already in r0.  (All values stored here fit in 16 bits, so a
/// single MOV.W suffices — no MOVT needed.)
fn store_word(imm: u32) -> [Thumb; 2] {
    [
        Thumb::W(movw_imm16(1, (imm & 0xFFFF) as u16)),
        Thumb::H(str_imm5(1, 0, 0)),
    ]
}

/// `MOV.W r1,#lo ; MOVT r1,#hi ; STR r1,[r0]` — store a full 32-bit immediate
/// to the MMIO address already in r0.
fn store_imm32(value: u32) -> [Thumb; 3] {
    [
        Thumb::W(movw_imm16(1, (value & 0xFFFF) as u16)),
        Thumb::W(movt_imm16(1, (value >> 16) as u16)),
        Thumb::H(str_imm5(1, 0, 0)),
    ]
}

/// Read-modify-write `*addr |= bit` (load r0=addr, r1=bit, r2=[r0], r2|=r1,
/// [r0]=r2). Used to ungate a single peripheral clock without clobbering the
/// other enable bits — mandatory for `RCC_AHBENR`, whose `SRAMEN`/`FLITFEN`
/// reset to 1 and would hang a program running from SRAM if overwritten.
fn enable_clock_bit(addr: u32, bit: u32) -> Vec<Thumb> {
    let mut s = Vec::new();
    s.extend(load_addr(0, addr));
    s.push(Thumb::W(movw_imm16(1, (bit & 0xFFFF) as u16)));
    s.push(Thumb::W(movt_imm16(1, (bit >> 16) as u16)));
    s.push(Thumb::H(ldr_imm5(2, 0, 0))); // r2 = *addr
    s.push(Thumb::H(orrs(2, 1))); // r2 |= bit
    s.push(Thumb::H(str_imm5(2, 0, 0))); // *addr = r2
    s
}

/// Build the full STM32F103 simulator bus (peripherals mapped), matching the
/// construction used by `stm32f1_mmio_diff`.
fn f103_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/stm32f103.yaml");
    let system_path = manifest_dir.join("../../configs/systems/stm32f103-bare.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build F103 sim bus: {e}"))
}

// ── 1. TIM2 update-generation (UG) event ───────────────────────────────────────
//
// Program (executed from SRAM, drives TIM2 over MMIO):
//   1. RCC_APB1ENR |= TIM2EN            — ungate the TIM2 clock (mandatory on
//                                         silicon before any TIM2 register works)
//   2. TIM2_PSC = 7                     — prescaler preload
//   3. TIM2_ARR = 0x1234               — auto-reload preload
//   4. TIM2_CNT = 0x5678               — seed the live counter NON-zero
//   5. TIM2_EGR = UG                    — generate an update event
//
// The update event (with CEN=0, so no free-running count to race) must, on
// both sim and silicon:
//   * reset CNT to 0           (the dynamics: a *live* 0x5678 is cleared)
//   * load the ARR/PSC shadows (ARR still reads 0x1234, PSC still reads 7)
//   * latch SR = 0x1F          (UIF + CC1..4IF: CNT=0 now matches every
//                               reset-zero CCRx in output-compare mode)
//
// CNT=0 is the load-bearing assertion: it proves UG cleared a counter we had
// just written non-zero — a register poke of CNT alone could never show this.
// The SR=0x1F assertion caught a real model gap (sim set UIF only); the fix
// models the UG-induced compare match. Both are now silicon-anchored.
#[thumb_oracle_test]
fn tim2_update_event() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    // 1. enable TIM2 clock
    prog.extend(load_addr(0, RCC_APB1ENR));
    prog.extend(store_word(RCC_APB1ENR_TIM2EN));
    // 2. PSC = 7
    prog.extend(load_addr(0, TIM2_PSC));
    prog.push(Thumb::H(movs_imm8(1, 7)));
    prog.push(Thumb::H(str_imm5(1, 0, 0)));
    // 3. ARR = 0x1234
    prog.extend(load_addr(0, TIM2_ARR));
    prog.extend(store_word(0x1234));
    // 4. CNT = 0x5678 (seed non-zero)
    prog.extend(load_addr(0, TIM2_CNT));
    prog.extend(store_word(0x5678));
    // 5. EGR.UG = 1 → update event
    prog.extend(load_addr(0, TIM2_EGR));
    prog.push(Thumb::H(movs_imm8(1, 1)));
    prog.push(Thumb::H(str_imm5(1, 0, 0)));

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[TIM2_CNT, TIM2_ARR, TIM2_PSC, TIM2_SR])
        .expect(|st| {
            st.assert_mem(TIM2_CNT, 0); // UG reset the live counter
            st.assert_mem(TIM2_ARR, 0x1234); // ARR preload intact
            st.assert_mem(TIM2_PSC, 7); // PSC preload intact
            st.assert_mem(TIM2_SR, TIM2_SR_AFTER_UG); // UIF + CC1..4IF (silicon)
        })
}

// ── 2. GPIOA atomic set/reset (BSRR / BRR, with BS-priority) ────────────────────
//
// Program (drives GPIOA over MMIO; pins stay in their reset floating-input
// mode — ODR is the output *latch* and reads back the written value regardless
// of pin direction, so no CRL/CRH setup is needed):
//   1. RCC_APB2ENR |= IOPAEN   — ungate the GPIOA clock (RMW)
//   2. ODR  = 0x0000           — clear the latch
//   3. BSRR = 0x0000_00FF      — BS sets bits 0..7        → ODR = 0x00FF
//   4. BSRR = 0x00F0_000F      — BR resets 4..7, BS sets 0..3 → ODR = 0x000F
//   5. BSRR = 0x0010_0010      — BS bit4 AND BR bit4: BS wins → ODR = 0x001F
//   6. BRR  = 0x0000_0003      — reset bits 0,1           → ODR = 0x001C
//
// Final ODR = 0x001C exercises BSRR-set, BSRR-reset, the BS-over-BR priority
// rule (step 5 is the load-bearing one — BR-wins would give 0x000F), and the
// F1-only BRR register, in a single executed program.
#[thumb_oracle_test]
fn gpioa_bsrr_set_reset() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(enable_clock_bit(RCC_APB2ENR, RCC_APB2ENR_IOPAEN));
    prog.extend(load_addr(0, GPIOA_ODR));
    prog.extend(store_imm32(0x0000_0000));
    prog.extend(load_addr(0, GPIOA_BSRR));
    prog.extend(store_imm32(0x0000_00FF));
    prog.extend(load_addr(0, GPIOA_BSRR));
    prog.extend(store_imm32(0x00F0_000F));
    prog.extend(load_addr(0, GPIOA_BSRR));
    prog.extend(store_imm32(0x0010_0010));
    prog.extend(load_addr(0, GPIOA_BRR));
    prog.extend(store_imm32(0x0000_0003));

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[GPIOA_ODR])
        .expect(|st| {
            st.assert_mem(GPIOA_ODR, 0x0000_001C);
        })
}

// ── 3. CRC-32 hardware compute ──────────────────────────────────────────────────
//
// Program (drives the CRC unit over MMIO):
//   1. RCC_AHBENR |= CRCEN   — ungate the CRC clock (RMW; must preserve
//                              SRAMEN/FLITFEN since we execute from SRAM)
//   2. CRC_CR = 1            — RESET: reload DR from the fixed init 0xFFFFFFFF
//   3. CRC_DR = 0x12345678   — feed word 1 through the polynomial engine
//   4. CRC_DR = 0x9ABCDEF0   — feed word 2
//   5. read CRC_DR           — the running CRC-32 (poly 0x04C11DB7, MSB-first,
//                              no in/out reflection, no final XOR)
//
// The expected value is the STM32 hardware CRC-32 of the two words; it's
// cross-validated against silicon by the `_diff` runner (the literal below is
// what both the model and the bench F103 produce). This exercises real
// combinational compute driven by executed code — not a static register poke.
#[thumb_oracle_test]
fn crc32_two_words() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(enable_clock_bit(RCC_AHBENR, RCC_AHBENR_CRCEN));
    prog.extend(load_addr(0, CRC_CR));
    prog.extend(store_imm32(0x0000_0001)); // RESET
    prog.extend(load_addr(0, CRC_DR));
    prog.extend(store_imm32(0x1234_5678)); // word 1
    prog.extend(load_addr(0, CRC_DR));
    prog.extend(store_imm32(0x9ABC_DEF0)); // word 2

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[CRC_DR])
        .expect(|st| {
            st.assert_mem(CRC_DR, CRC32_TWO_WORDS);
        })
}

/// STM32 hardware CRC-32 of `[0x12345678, 0x9ABCDEF0]` from the reset init
/// (0xFFFFFFFF). Pinned from the model and cross-checked against bench F103
/// silicon by `crc32_two_words_diff`.
const CRC32_TWO_WORDS: u32 = 0x7D24_A31B;
