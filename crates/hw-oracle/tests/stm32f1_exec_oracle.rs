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
    assemble, bx, cmp_reg, cpsie_i, it, ldr_imm5, movs_imm8, movt_imm16, movw_imm16, orrs,
    str_imm5, Thumb, ThumbOracleCase, INIT_SP, PROG_BASE_HW,
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
/// RCC AHB clock-enable (RCC + 0x14) and the CRC/DMA1 enable bits.
const RCC_AHBENR: u32 = 0x4002_1014;
const RCC_AHBENR_CRCEN: u32 = 1 << 6;
const RCC_AHBENR_DMA1EN: u32 = 1 << 0;

/// DMA1 (RM0008 §13): controller + channel-1 register block.
const DMA1_BASE: u32 = 0x4002_0000;
const DMA1_ISR: u32 = DMA1_BASE; // interrupt status (GIF/TCIF/HTIF/TEIF ×7)
const DMA1_CCR1: u32 = DMA1_BASE + 0x08; // channel-1 config
const DMA1_CNDTR1: u32 = DMA1_BASE + 0x0C; // channel-1 transfer count
const DMA1_CPAR1: u32 = DMA1_BASE + 0x10; // channel-1 "peripheral" address
const DMA1_CMAR1: u32 = DMA1_BASE + 0x14; // channel-1 "memory" address

/// Channel-1 CCR bits used by the mem-to-mem oracle.
const CCR_EN: u32 = 1 << 0;
const CCR_DIR: u32 = 1 << 4; // read-from-memory (required with MEM2MEM)
const CCR_PINC: u32 = 1 << 6;
const CCR_MINC: u32 = 1 << 7;
const CCR_MEM2MEM: u32 = 1 << 14;

/// SRAM scratch buffers for the DMA copy (well clear of the program at
/// PROG_BASE_HW=0x2000_2000 and the stack growing down from INIT_SP).
const DMA_SRC: u32 = 0x2000_0100;
const DMA_DST: u32 = 0x2000_0200;

/// GPIOA (RM0008): config + output-data + atomic set/reset registers.
const GPIOA_BASE: u32 = 0x4001_0800;
const GPIOA_CRL: u32 = GPIOA_BASE; // pin config, pins 0..7
const GPIOA_CRH: u32 = GPIOA_BASE + 0x04; // pin config, pins 8..15
const GPIOA_ODR: u32 = GPIOA_BASE + 0x0C; // output data
const GPIOA_BSRR: u32 = GPIOA_BASE + 0x10; // atomic set (lo16) / reset (hi16)
const GPIOA_BRR: u32 = GPIOA_BASE + 0x14; // atomic reset (lo16)

/// AFIO EXTICR1 (line-source mux for EXTI 0..3); upper 16 bits reserved.
const AFIO_EXTICR1: u32 = 0x4001_0008;

/// DBGMCU control register (Cortex-M debug-MCU block); bits [4:3] reserved.
const DBGMCU_CR: u32 = 0xE004_2004;

/// CRC unit (RM0008): data register + control.
const CRC_BASE: u32 = 0x4002_3000;
const CRC_DR: u32 = CRC_BASE; // data in / CRC result out
const CRC_IDR: u32 = CRC_BASE + 0x04; // independent data register (8-bit on F1)
const CRC_CR: u32 = CRC_BASE + 0x08; // control (RESET = bit 0)

/// AFIO (RM0008 §9): the remap register. AFIOEN is APB2ENR bit 0.
const RCC_APB2ENR_AFIOEN: u32 = 1 << 0;
const AFIO_MAPR: u32 = 0x4001_0004;

/// IWDG (RM0008 §19): independent watchdog. PR/RLR are write-protected until KR
/// receives the 0x5555 unlock code; this oracle pins that they stay at reset
/// without it. NB: never write the 0xCCCC start key — it arms the watchdog,
/// which then resets the chip and cannot be stopped except by a power cycle.
const IWDG_BASE: u32 = 0x4000_3000;
const IWDG_PR: u32 = IWDG_BASE + 0x04; // prescaler (write-protected)
const IWDG_RLR: u32 = IWDG_BASE + 0x08; // reload (write-protected)

/// EXTI (RM0008 §10): software-interrupt + pending registers.
const EXTI_BASE: u32 = 0x4001_0400;
const EXTI_IMR: u32 = EXTI_BASE; // interrupt mask
const EXTI_SWIER: u32 = EXTI_BASE + 0x10; // software interrupt event
const EXTI_PR: u32 = EXTI_BASE + 0x14; // pending (rc_w1)

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

    // The full-chip sim path wires the Cortex-M system block (NVIC @
    // 0xE000E100, SCB @ 0xE000ED00, DWT @ 0xE000_1000) when it builds the CPU
    // (run_capture → configure_cortex_m), so NVIC/SCB/DWT MMIO is available to
    // every oracle and interrupt-delivery oracles get a CPU sharing the bus's
    // NVIC/VTOR.
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

// ── 4. DMA1 memory-to-memory transfer ───────────────────────────────────────────
//
// The first oracle to exercise an *autonomous* engine: the program arms a DMA
// mem-to-mem copy and stops; the DMA then moves the bytes on its own. On
// silicon it runs concurrently and finishes long before the breakpoint halt;
// in sim the harness's `settle_ticks` advances the engine (one byte/tick) to
// completion after the program settles.
//
// Program:
//   1. RCC_AHBENR |= DMA1EN          — ungate the DMA1 clock (RMW)
//   2. fill DMA_SRC with two known words; zero DMA_DST (so a no-op copy is
//      detectable, not a stale match)
//   3. CCR1 = 0                       — disable the channel before reconfig
//   4. CMAR1 = DMA_SRC               — memory (source) address
//   5. CPAR1 = DMA_DST              — "peripheral" (destination) address
//   6. CNDTR1 = 8                    — eight byte-elements
//   7. CCR1 = MEM2MEM|MINC|PINC|DIR|EN — arm an 8-bit mem-to-mem copy
//
// After settle, DMA_DST must equal DMA_SRC, CNDTR1 must read 0 (all elements
// moved), and ISR must show GIF1|TCIF1|HTIF1 (0x7) for channel 1.
#[thumb_oracle_test]
fn dma1_mem_to_mem() -> ThumbOracleCase {
    const W0: u32 = 0xDEAD_BEEF;
    const W1: u32 = 0xCAFE_B0BA;
    const CCR_CFG: u32 = CCR_MEM2MEM | CCR_MINC | CCR_PINC | CCR_DIR | CCR_EN;

    let mut prog: Vec<Thumb> = Vec::new();
    // 1. enable DMA1 clock
    prog.extend(enable_clock_bit(RCC_AHBENR, RCC_AHBENR_DMA1EN));
    // 2. fill source, zero destination
    prog.extend(load_addr(0, DMA_SRC));
    prog.extend(store_imm32(W0));
    prog.extend(load_addr(0, DMA_SRC + 4));
    prog.extend(store_imm32(W1));
    prog.extend(load_addr(0, DMA_DST));
    prog.extend(store_imm32(0));
    prog.extend(load_addr(0, DMA_DST + 4));
    prog.extend(store_imm32(0));
    // 3. disable channel before reconfiguring
    prog.extend(load_addr(0, DMA1_CCR1));
    prog.extend(store_imm32(0));
    // 4-6. addresses + count
    prog.extend(load_addr(0, DMA1_CMAR1));
    prog.extend(store_imm32(DMA_SRC));
    prog.extend(load_addr(0, DMA1_CPAR1));
    prog.extend(store_imm32(DMA_DST));
    prog.extend(load_addr(0, DMA1_CNDTR1));
    prog.extend(store_imm32(8));
    // 7. arm the transfer
    prog.extend(load_addr(0, DMA1_CCR1));
    prog.extend(store_imm32(CCR_CFG));

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .settle_ticks(16) // > 8 elements; extra ticks are no-ops once idle
        .capture_mem(&[DMA_DST, DMA_DST + 4, DMA1_CNDTR1, DMA1_ISR])
        .expect(|st| {
            st.assert_mem(DMA_DST, W0); // engine copied word 0
            st.assert_mem(DMA_DST + 4, W1); // engine copied word 1
            st.assert_mem(DMA1_CNDTR1, 0); // all 8 elements moved
            st.assert_mem(DMA1_ISR, 0x0000_0007); // GIF1 | TCIF1 | HTIF1
        })
}

// ── 5. EXTI software-interrupt trigger (SWIER → PR), then PR clear ───────────────
//
// EXTI needs no clock enable on F1 (the block sits directly on the bus; only the
// AFIO EXTICR muxes need AFIOEN, which this oracle doesn't touch). Pure register
// dynamics, no external signal:
//   1. IMR  = 0x05            — unmask lines 0 and 2
//   2. SWIER = 0x05           — software-trigger lines 0 and 2 → PR sets bits 0,2
//   3. PR   = 0x01            — rc_w1: clear pending line 0
//
// After: PR reads 0x04 (line 2 still pending, line 0 cleared). SWIER reads 0x04
// too — on silicon clearing a PR bit also clears the matching SWIER bit (RM0008
// §10.3.6). Capturing SWIER pins that coupling.
#[thumb_oracle_test]
fn exti_swier_sets_and_clears_pr() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(load_addr(0, EXTI_IMR));
    prog.extend(store_imm32(0x0000_0005));
    prog.extend(load_addr(0, EXTI_SWIER));
    prog.extend(store_imm32(0x0000_0005));
    prog.extend(load_addr(0, EXTI_PR));
    prog.extend(store_imm32(0x0000_0001)); // rc_w1: clear line 0

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[EXTI_PR, EXTI_SWIER])
        .expect(|st| {
            st.assert_mem(EXTI_PR, 0x0000_0004); // line 2 still pending
            st.assert_mem(EXTI_SWIER, 0x0000_0004); // SWIER bit0 cleared with PR bit0
        })
}

// ── 6. AFIO MAPR reserved-bit masking ───────────────────────────────────────────
//
// AFIO_MAPR has implemented remap bits [20:0], a write-only SWJ_CFG field
// [26:24], and reserved bits elsewhere. Silicon reads reserved bits back as 0;
// a naive model that stores the written word verbatim does not.
//
// Program:
//   1. RCC_APB2ENR |= AFIOEN          — ungate the AFIO clock (RMW)
//   2. MAPR = 0x0820_0004             — set reserved bits 27 and 21, plus the
//                                       USART1_REMAP bit (2). **SWJ_CFG [26:24]
//                                       is left 0** (full SWJ / SWD stays up —
//                                       writing 0b111 there would disable SWD
//                                       and drop the debugger).
//
// Read back (masking out the write-only/undefined SWJ_CFG field): the reserved
// bits must read 0 and only the remap bit survive → 0x0000_0004.
#[thumb_oracle_test]
fn afio_mapr_reserved_bits_read_zero() -> ThumbOracleCase {
    // Reserved bits 27 (0x0800_0000) and 21 (0x0020_0000) + USART1_REMAP (bit 2).
    // Deliberately NO bits in 24..26 (SWJ_CFG) — see the safety note above.
    const MAPR_WRITE: u32 = 0x0820_0004;
    // SWJ_CFG [26:24] reads "undefined" (RM0008); exclude it from the check.
    const SWJ_CFG: u32 = 0x0700_0000;

    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(enable_clock_bit(RCC_APB2ENR, RCC_APB2ENR_AFIOEN));
    prog.extend(load_addr(0, AFIO_MAPR));
    prog.extend(store_imm32(MAPR_WRITE));

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[AFIO_MAPR])
        .expect(|st| {
            let mapr = st.read_mem(AFIO_MAPR);
            assert_eq!(
                mapr & !SWJ_CFG,
                0x0000_0004,
                "AFIO_MAPR reserved bits must read 0 (got 0x{mapr:08X})"
            );
        })
}

// ── 7. IWDG write-protected PR/RLR ──────────────────────────────────────────────
//
// IWDG_PR and IWDG_RLR are write-protected: writes are dropped unless KR has
// first received the 0x5555 unlock code (RM0008 §19.4). This oracle pins the
// protection — a PR/RLR write WITHOUT the key leaves them at their reset values
// (PR=0, RLR=0xFFF). Silicon-confirmed on the bench F103.
//
// (Only the negative path is pinned. The positive path — PR/RLR latching after
// the 0x5555 unlock — additionally needs the IWDG clock domain running, which
// on F103 means starting the watchdog (KR=0xCCCC, which then resets the chip)
// or enabling the LSI first; not worth arming a watchdog reset to assert. The
// 0xCCCC start key is never written here.)
#[thumb_oracle_test]
fn iwdg_pr_rlr_write_protected_without_key() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(load_addr(0, IWDG_PR));
    prog.extend(store_imm32(0x5)); // protected — no prior 0x5555
    prog.extend(load_addr(0, IWDG_RLR));
    prog.extend(store_imm32(0x123)); // protected — no prior 0x5555

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[IWDG_PR, IWDG_RLR])
        .expect(|st| {
            st.assert_mem(IWDG_PR, 0x0); // write dropped → reset value
            st.assert_mem(IWDG_RLR, 0xFFF); // write dropped → reset value
        })
}

// ── 8. CRC_IDR is 8-bit on STM32F1 ──────────────────────────────────────────────
//
// The CRC independent data register is a general-purpose scratch byte. On
// STM32F1 it is 8-bit: bits [31:8] are reserved and read 0 (RM0008 §6.4.2).
// (On L4+ the same register is 32-bit — hence the model needs a width flag.)
//
// Program: enable the CRC clock, write a full 32-bit word to IDR, read it back.
// Silicon keeps only the low byte → 0x78.
#[thumb_oracle_test]
fn crc_idr_is_8bit_on_f1() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(enable_clock_bit(RCC_AHBENR, RCC_AHBENR_CRCEN));
    prog.extend(load_addr(0, CRC_IDR));
    prog.extend(store_imm32(0x1234_5678));

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[CRC_IDR])
        .expect(|st| {
            st.assert_mem(CRC_IDR, 0x0000_0078); // only the low byte survives
        })
}

// ── 9. AFIO EXTICR1 upper half reserved ─────────────────────────────────────────
//
// Each EXTICR holds four 4-bit line-source fields in bits [15:0]; bits [31:16]
// are reserved and read 0 (RM0008 §9.4.3). Same masking class as MAPR (#17).
// Program: enable AFIO clock, write a reserved upper bit + a valid nibble.
#[thumb_oracle_test]
fn afio_exticr1_upper_half_reads_zero() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(enable_clock_bit(RCC_APB2ENR, RCC_APB2ENR_AFIOEN));
    prog.extend(load_addr(0, AFIO_EXTICR1));
    prog.extend(store_imm32(0x0001_0002)); // reserved bit 16 + EXTI0→port C
    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[AFIO_EXTICR1])
        .expect(|st| st.assert_mem(AFIO_EXTICR1, 0x0000_0002))
}

// ── 10. GPIOA CRL/CRH config round-trip ─────────────────────────────────────────
//
// CRL/CRH are plain 32-bit R/W config registers (4 bits per pin, all bits
// implemented — no reserved fields). A validation oracle: the written config
// must read back verbatim on both sim and silicon.
#[thumb_oracle_test]
fn gpioa_crl_crh_round_trip() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(enable_clock_bit(RCC_APB2ENR, RCC_APB2ENR_IOPAEN));
    prog.extend(load_addr(0, GPIOA_CRL));
    prog.extend(store_imm32(0x3333_3333)); // all low pins: output 50MHz push-pull
    prog.extend(load_addr(0, GPIOA_CRH));
    prog.extend(store_imm32(0x4848_4848)); // mix of floating-input / output
    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[GPIOA_CRL, GPIOA_CRH])
        .expect(|st| {
            st.assert_mem(GPIOA_CRL, 0x3333_3333);
            st.assert_mem(GPIOA_CRH, 0x4848_4848);
        })
}

// ── 11. DBGMCU_CR round-trip (incl. RM-reserved bits [4:3]) ──────────────────────
//
// A validation oracle with a silicon surprise: RM0008 §31.16.2 documents
// DBGMCU_CR bits [4:3] as reserved, but the bench F103 reads them back exactly
// as written (0x1F, not 0x07) — they are plain R/W storage on this silicon. The
// model's verbatim store already matches; this oracle pins the actual behaviour
// so a future "mask the reserved bits" change can't silently break it.
// (Only the safe low/reserved bits are touched — no TRACE_IOEN, so the SWD pins
// are untouched and the debugger stays connected.)
#[thumb_oracle_test]
fn dbgmcu_cr_round_trip() -> ThumbOracleCase {
    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(load_addr(0, DBGMCU_CR));
    prog.extend(store_imm32(0x0000_001F)); // DBG_SLEEP/STOP/STANDBY + RM-reserved 3,4
    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[DBGMCU_CR])
        .expect(|st| st.assert_mem(DBGMCU_CR, 0x0000_001F)) // bits 3,4 stick on silicon
}

// ── 12. NVIC ISER/ICER set-enable / clear-enable ────────────────────────────────
//
// The NVIC interrupt-enable bank uses banked set/clear registers (ARMv7-M B3.4):
// ISERx (write 1 = enable, write 0 = ignored, read = current enable state) and
// ICERx (write 1 = disable). This pins that pair — the enable machinery that
// real interrupt delivery rides on — without yet taking an interrupt.
//
// Program: enable IRQ6 (EXTI0) + IRQ28 (TIM2) via ISER0, then disable only IRQ6
// via ICER0. ISER0 must read back with IRQ28 still enabled, IRQ6 cleared.
#[thumb_oracle_test]
fn nvic_iser_icer_enable_disable() -> ThumbOracleCase {
    const NVIC_ISER0: u32 = 0xE000_E100;
    const NVIC_ICER0: u32 = 0xE000_E180;
    const IRQ6: u32 = 1 << 6; // EXTI0
    const IRQ28: u32 = 1 << 28; // TIM2

    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(load_addr(0, NVIC_ISER0));
    prog.extend(store_imm32(IRQ6 | IRQ28)); // enable both
    prog.extend(load_addr(0, NVIC_ICER0));
    prog.extend(store_imm32(IRQ6)); // clear-enable IRQ6 only

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .capture_mem(&[NVIC_ISER0])
        .expect(|st| st.assert_mem(NVIC_ISER0, IRQ28)) // IRQ28 enabled, IRQ6 cleared
}

// ── 13. EXTI0 interrupt delivery (the first real exception-entry oracle) ─────────
//
// Lays out [vector table @ load base][ISR][main]. `main` relocates VTOR to the
// table, unmasks EXTI line 0, enables IRQ6 in the NVIC, clears PRIMASK, and
// software-triggers the line. The CPU must vector to the ISR (exception entry +
// stacking), which writes a RAM marker and clears EXTI_PR, then `BX LR` returns
// (unstacking) into `main`, which settles at the terminator.
//
// Exercises the whole delivery path — VTOR relocation, NVIC enable, vectoring,
// stacking/unstacking, exception return — that the static oracles never touch.
// The ISR is idempotent (a marker store, not a counter) so the sim's
// tick-granular level-source re-pend (it may run the ISR an extra time before
// EXTI_PR clears) produces the same final state as silicon's single entry.

/// Build [32-entry vector table][isr][main] and return the program + the byte
/// offset of `main` (the entry point). `exc_num` is the exception number
/// (IRQ + 16) whose vector points at the ISR.
fn interrupt_program(isr: &[Thumb], main: &[Thumb], exc_num: usize) -> (Vec<Thumb>, u32) {
    const TABLE_ENTRIES: usize = 32; // 128 bytes — VTOR is 128-byte aligned at the load base
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

#[thumb_oracle_test]
fn exti0_interrupt_delivery() -> ThumbOracleCase {
    const VTOR_REG: u32 = 0xE000_ED08; // SCB->VTOR
    const NVIC_ISER0: u32 = 0xE000_E100;
    const MARKER: u32 = 0x2000_0300; // RAM marker the ISR writes
    const MARKER_VALUE: u32 = 0xABCD_1234;
    const EXTI0_EXC: usize = 16 + 6; // IRQ6 → exception 22

    // ISR: write the marker, clear the EXTI pending bit, return.
    let mut isr: Vec<Thumb> = Vec::new();
    isr.extend(load_addr(0, MARKER));
    isr.extend(store_imm32(MARKER_VALUE));
    isr.extend(load_addr(0, EXTI_PR));
    isr.extend(store_imm32(0x1)); // rc_w1: clear pending line 0
    isr.push(Thumb::H(bx(14))); // BX LR — exception return

    // main: relocate VTOR, zero the marker, enable + trigger the interrupt.
    let mut main: Vec<Thumb> = Vec::new();
    main.extend(load_addr(0, VTOR_REG));
    main.extend(store_imm32(PROG_BASE_HW)); // VTOR = table base
    main.extend(load_addr(0, MARKER));
    main.extend(store_imm32(0x0)); // clear marker → proves the ISR set it
    main.extend(load_addr(0, EXTI_IMR));
    main.extend(store_imm32(0x1)); // unmask EXTI line 0
    main.extend(load_addr(0, NVIC_ISER0));
    main.extend(store_imm32(1 << 6)); // enable IRQ6 (EXTI0)
    main.push(Thumb::H(cpsie_i())); // clear PRIMASK
    main.extend(load_addr(0, EXTI_SWIER));
    main.extend(store_imm32(0x1)); // software-trigger line 0 → IRQ fires

    let (prog, entry) = interrupt_program(&isr, &main, EXTI0_EXC);

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .entry_offset(entry)
        .live_peripherals(true)
        .capture_mem(&[MARKER, EXTI_PR])
        .expect(|st| {
            st.assert_mem(MARKER, MARKER_VALUE); // the ISR ran (exception delivered)
            st.assert_mem(EXTI_PR, 0); // the ISR cleared the pending bit
        })
}

// ── 14. DWT cycle counter advances (timed-peripheral mechanism, roadmap P2) ──────
//
// Enables the DWT cycle counter (DEMCR.TRCENA + DWT_CTRL.CYCCNTENA), resets it,
// then reads it back after a few instructions and records a self-relative
// boolean: CYCCNT != 0. The absolute cycle count diverges between sim (which is
// NOT cycle-accurate — its DWT advances one tick per executed instruction) and
// silicon (true core cycles + wait states), so an exact diff of CYCCNT would be
// meaningless. The boolean is invariant — the counter advances on both — so this
// pins the DWT *mechanism* (enable → count → read), not timing fidelity. True
// cycle-accuracy is out of scope for this simulator; see roadmap.md P2.
#[thumb_oracle_test]
fn dwt_cyccnt_advances() -> ThumbOracleCase {
    const DEMCR: u32 = 0xE000_EDFC;
    const DEMCR_TRCENA: u32 = 1 << 24;
    const DWT_CTRL: u32 = 0xE000_1000;
    const DWT_CYCCNT: u32 = 0xE000_1004;
    const DWT_CTRL_CYCCNTENA: u32 = 1 << 0;
    const MARKER: u32 = 0x2000_0304;
    const COND_NE: u8 = 0b0001;

    let mut prog: Vec<Thumb> = Vec::new();
    prog.extend(load_addr(0, DEMCR));
    prog.extend(store_imm32(DEMCR_TRCENA)); // enable the trace/DWT block
    prog.extend(load_addr(0, DWT_CTRL));
    prog.extend(store_imm32(DWT_CTRL_CYCCNTENA)); // enable CYCCNT
    prog.extend(load_addr(0, DWT_CYCCNT));
    prog.extend(store_imm32(0)); // reset the counter
    prog.extend(load_addr(0, DWT_CYCCNT));
    prog.push(Thumb::H(ldr_imm5(3, 0, 0))); // r3 = CYCCNT (nonzero — cycles elapsed)
                                            // marker = (r3 != 0) ? 1 : 0, via an IT NE block (no branches).
    prog.push(Thumb::H(movs_imm8(1, 0))); // r1 = 0
    prog.push(Thumb::H(movs_imm8(2, 0))); // r2 = 0
    prog.push(Thumb::H(cmp_reg(3, 2))); // Z = (CYCCNT == 0)
    prog.push(Thumb::H(it(COND_NE, 0x8))); // IT NE
    prog.push(Thumb::H(movs_imm8(1, 1))); // r1 = 1 iff CYCCNT != 0
                                          // r3 holds the absolute (divergent) cycle count — clear it so the final
                                          // register state the _diff compares is deterministic. NZCV stays equal:
                                          // the prior CMP gives Z=0/C=1 on both (both counts are positive-nonzero).
    prog.push(Thumb::H(movs_imm8(3, 0)));
    prog.extend(load_addr(0, MARKER));
    prog.push(Thumb::H(str_imm5(1, 0, 0))); // marker = r1

    ThumbOracleCase::mixed(&prog)
        .sim_bus(f103_bus)
        .live_peripherals(true) // tick the DWT each step so CYCCNT advances in sim
        .capture_mem(&[MARKER])
        .expect(|st| st.assert_mem(MARKER, 1)) // counter advanced on both sim and silicon
}
