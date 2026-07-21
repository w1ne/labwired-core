// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
//! STM32F103 conformance firmware.
//!
//! Drives each peripheral through a realistic register sequence and writes an
//! observable-state **digest** to a fixed RAM block. The same ELF runs on the
//! simulator (full-chip `Machine`) and on real silicon; the harness
//! (`crates/hw-oracle/tests/f103_conformance.rs`) diffs the two digests. A
//! mismatch in a deterministic field is a real modeling gap; fields that would
//! be timing- or analog-dependent are reduced to invariant flags here so the
//! diff has no false positives.
//!
//! Layout convention: `VERDICT[0]` is the DONE sentinel (written last); the
//! harness polls it to know the run finished. `VERDICT[1..]` are per-test digest
//! words — see `IDX_*`.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};

/// Fixed RAM block (well below the stack at 0x2000_5000, above the DMA scratch).
const VERDICT: u32 = 0x2000_3000;
/// Written to `VERDICT[0]` after every test completes.
const DONE_MAGIC: u32 = 0xC0DE_F103;

// ── Digest indices ──────────────────────────────────────────────────────────
const IDX_DONE: usize = 0;
const IDX_GPIO_ODR: usize = 1;
const IDX_TIM2_SR: usize = 2;
const IDX_TIM2_CNT: usize = 3;
const IDX_CRC: usize = 4;
const IDX_EXTI_PR: usize = 5;
const IDX_EXTI_SWIER: usize = 6;
const IDX_DMA_D0: usize = 7;
const IDX_DMA_D1: usize = 8;
const IDX_DMA_CNDTR: usize = 9;
const IDX_DMA_ISR: usize = 10;
const VERDICT_WORDS: usize = 32;

// ── Register map (RM0008) ─────────────────────────────────────────────────────
const RCC_AHBENR: u32 = 0x4002_1014;
const RCC_APB2ENR: u32 = 0x4002_1018;
const RCC_APB1ENR: u32 = 0x4002_101C;

const GPIOA: u32 = 0x4001_0800;
const TIM2: u32 = 0x4000_0000;
const CRC: u32 = 0x4002_3000;
const EXTI: u32 = 0x4001_0400;
const DMA1: u32 = 0x4002_0000;

const DMA_SRC: u32 = 0x2000_2000;
const DMA_DST: u32 = 0x2000_2100;

#[inline(always)]
unsafe fn wr(addr: u32, val: u32) {
    write_volatile(addr as *mut u32, val);
}
#[inline(always)]
unsafe fn rd(addr: u32) -> u32 {
    read_volatile(addr as *const u32)
}
/// Enable a peripheral clock and read the enable register back — the dummy read
/// covers the RCC clock-enable-to-access delay (RM0008 erratum). Without it,
/// accessing the peripheral on real silicon immediately after the enable can
/// bus-fault (the sim has no such delay, so this also keeps the two in step).
#[inline(always)]
unsafe fn enable_clock(rcc_reg: u32, bits: u32) {
    wr(rcc_reg, rd(rcc_reg) | bits);
    let _ = rd(rcc_reg);
    settle();
}

/// Short busy-wait. Real silicon needs a few cycles for a peripheral to settle
/// after a clock-enable or a self-clearing control write (RCC enable delay, CRC
/// reset completion, …); HAL code achieves this incidentally through its
/// surrounding instructions. The sim models no such latency — this keeps the
/// tightly-coded conformance firmware correct on both.
#[inline(never)]
fn settle() {
    for _ in 0..8 {
        unsafe { core::arch::asm!("nop") };
    }
}

/// Diagnostic fault marker written to `VERDICT[0]` if the firmware faults — the
/// harness reads it on timeout to distinguish a crash from a hang.
const FAULT_MAGIC: u32 = 0xDEAD_FA17;

#[no_mangle]
pub extern "C" fn HardFaultHandler() -> ! {
    unsafe { digest(IDX_DONE, FAULT_MAGIC) };
    halt_forever()
}

#[no_mangle]
pub extern "C" fn DefaultHandler() -> ! {
    unsafe { digest(IDX_DONE, FAULT_MAGIC) };
    halt_forever()
}

#[inline(never)]
fn halt_forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[inline(always)]
unsafe fn digest(idx: usize, val: u32) {
    wr(VERDICT + (idx as u32) * 4, val);
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        for i in 0..VERDICT_WORDS {
            digest(i, 0);
        }
        test_gpio();
        test_tim2();
        test_crc();
        test_exti();
        test_dma();
        digest(IDX_DONE, DONE_MAGIC); // sentinel — must be last
    }
    halt_forever()
}

/// GPIOA atomic set/reset (BSRR/BRR with BS-over-BR priority) → ODR = 0x001C.
unsafe fn test_gpio() {
    enable_clock(RCC_APB2ENR, 1 << 2); // IOPAEN
    wr(GPIOA + 0x0C, 0x0000); // ODR = 0
    wr(GPIOA + 0x10, 0x0000_00FF); // BSRR: set 0..7
    wr(GPIOA + 0x10, 0x00F0_000F); // BSRR: reset 4..7, set 0..3 → 0x000F
    wr(GPIOA + 0x10, 0x0010_0010); // BSRR: BS bit4 wins over BR bit4 → 0x001F
    wr(GPIOA + 0x14, 0x0000_0003); // BRR: reset 0,1 → 0x001C
    digest(IDX_GPIO_ODR, rd(GPIOA + 0x0C));
}

/// TIM2 update event: SR latches UIF + CC1..4IF (0x1F), CNT resets to 0.
unsafe fn test_tim2() {
    enable_clock(RCC_APB1ENR, 1 << 0); // TIM2EN
    wr(TIM2 + 0x28, 7); // PSC
    wr(TIM2 + 0x2C, 0x1234); // ARR
    wr(TIM2 + 0x24, 0x5678); // CNT (seed non-zero)
    wr(TIM2 + 0x14, 0x1); // EGR.UG
    settle(); // status flags need a cycle to latch on silicon
    digest(IDX_TIM2_SR, rd(TIM2 + 0x10));
    digest(IDX_TIM2_CNT, rd(TIM2 + 0x24));
}

/// Hardware CRC-32 of two words.
unsafe fn test_crc() {
    enable_clock(RCC_AHBENR, 1 << 6); // CRCEN
    wr(CRC + 0x08, 1); // CR.RESET
    settle();
    wr(CRC, 0x1234_5678); // DR
    wr(CRC, 0x9ABC_DEF0);
    digest(IDX_CRC, rd(CRC));
}

/// EXTI software-trigger lines 0+2, then clear line 0 → PR=0x4, SWIER=0x4.
unsafe fn test_exti() {
    // Start from a clean slate: no edge triggers, clear any stale pending left
    // by earlier tests (EXTI lines mux to GPIO pins, which the GPIO test drove).
    wr(EXTI + 0x08, 0); // RTSR = 0
    wr(EXTI + 0x0C, 0); // FTSR = 0
    wr(EXTI + 0x14, 0x000F_FFFF); // PR rc_w1: clear all lines
    settle();
    wr(EXTI, 0x5); // IMR lines 0,2
    wr(EXTI + 0x10, 0x5); // SWIER -> PR
    wr(EXTI + 0x14, 0x1); // PR rc_w1: clear line 0
    settle();
    digest(IDX_EXTI_PR, rd(EXTI + 0x14));
    digest(IDX_EXTI_SWIER, rd(EXTI + 0x10));
}

/// DMA1 channel-1 memory-to-memory copy (8 bytes), polled to completion.
unsafe fn test_dma() {
    enable_clock(RCC_AHBENR, 1 << 0); // DMA1EN
    wr(DMA_SRC, 0xDEAD_BEEF);
    wr(DMA_SRC + 4, 0xCAFE_B0BA);
    wr(DMA_DST, 0);
    wr(DMA_DST + 4, 0);
    wr(DMA1 + 0x08, 0); // CCR1 = 0 (disable before reconfig)
    wr(DMA1 + 0x14, DMA_SRC); // CMAR1 (source)
    wr(DMA1 + 0x10, DMA_DST); // CPAR1 (destination)
    wr(DMA1 + 0x0C, 8); // CNDTR1 = 8 bytes
                        // MEM2MEM | MINC | PINC | DIR | EN
    wr(
        DMA1 + 0x08,
        (1 << 14) | (1 << 7) | (1 << 6) | (1 << 4) | (1 << 0),
    );
    // Poll TCIF1 (ISR bit 1) with a bounded timeout.
    let mut guard: u32 = 0;
    while (rd(DMA1) & (1 << 1)) == 0 && guard < 1_000_000 {
        guard += 1;
    }
    digest(IDX_DMA_D0, rd(DMA_DST));
    digest(IDX_DMA_D1, rd(DMA_DST + 4));
    digest(IDX_DMA_CNDTR, rd(DMA1 + 0x0C));
    digest(IDX_DMA_ISR, rd(DMA1));
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    halt_forever()
}
