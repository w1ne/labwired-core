//! Checks shared by the STM32F4 fixtures (F401, F407, F411).
//!
//! The F4 parts share DMA (RM0090 §10 stream controller), NVIC/EXTI, and the
//! TIM1 advanced-control timer, so these checks are parameterized by base
//! address rather than duplicated per part.
//!
//! `wdt` (IWDG) and `rtc` are reported but carry no clock gate: IWDG runs off
//! LSI and RTC off the backup domain (RCC_BDCR.RTCEN), neither of which is an
//! APB/AHB peripheral-enable bit. Every other class is gate-proved: the check
//! reads the peripheral dead while its RCC bit is off before enabling it.

use crate::{rd32, wr32};
use core::ptr::read_volatile;

// DMA2 stream 0 register block (RM0090 §10.5): streams start at 0x10, stride 0x18.
const DMA2_LISR_OFFSET: u32 = 0x00; // interrupt status, streams 0..3
const DMA2_LIFCR_OFFSET: u32 = 0x08; // flag clear, streams 0..3
const DMA2_S0CR_OFFSET: u32 = 0x10;
const DMA2_S0NDTR_OFFSET: u32 = 0x14;
const DMA2_S0PAR_OFFSET: u32 = 0x18;
const DMA2_S0M0AR_OFFSET: u32 = 0x1C;
// LISR/LIFCR stream-0 flag bits (RM0090 §10.5.1, confirmed against the SVD):
// FEIF0=bit0, DMEIF0=bit2, TEIF0=bit3, HTIF0=bit4, TCIF0=bit5.
const DMA_TCIF0: u32 = 1 << 5;

// Source and destination buffers for the memory-to-memory transfer. `static mut`
// with volatile access: the DMA engine writes DST behind the compiler's back.
static mut DMA_SRC: [u32; 4] = [0xA5A5_0001, 0xA5A5_0002, 0xA5A5_0003, 0xA5A5_0004];
static mut DMA_DST: [u32; 4] = [0; 4];

/// dma: DMA2 memory-to-memory (RM0368 §9.3.3 — mem2mem is DMA2-only; a DIR=10
/// enable on DMA1 is rejected by the model, matching silicon). Gated on
/// RCC_AHB1ENR.DMA2EN (bit 22): while the gate is off the stream registers are
/// unclocked, so an SxNDTR write is dropped and reads back 0. After enabling,
/// program a 4-word transfer, set EN, and poll LISR.TCIF0 — then verify the
/// destination buffer actually holds the source words, which proves the engine
/// moved data rather than merely raising a flag.
pub fn check_dma(
    rcc_ahb1enr: u32,
    dma2_base: u32,
    src: u32,
    dst: u32,
) -> Result<(), &'static [u8]> {
    let s0cr = dma2_base + DMA2_S0CR_OFFSET;
    let s0ndtr = dma2_base + DMA2_S0NDTR_OFFSET;
    let s0par = dma2_base + DMA2_S0PAR_OFFSET;
    let s0m0ar = dma2_base + DMA2_S0M0AR_OFFSET;
    let lisr = dma2_base + DMA2_LISR_OFFSET;
    let lifcr = dma2_base + DMA2_LIFCR_OFFSET;

    // Gate OFF out of reset → write dropped, register reads back 0.
    wr32(s0ndtr, 4);
    if rd32(s0ndtr) != 0 {
        return Err(b"dma-gated");
    }
    wr32(rcc_ahb1enr, rd32(rcc_ahb1enr) | (1 << 22)); // DMA2EN

    wr32(s0cr, 0); // EN=0 before reprogramming (RM0090 §10.3.17)
    wr32(lifcr, 0x3D); // clear all stream-0 flags
    wr32(s0par, src); // in mem2mem the peripheral port is the source
    wr32(s0m0ar, dst);
    wr32(s0ndtr, 4);
    if rd32(s0ndtr) != 4 {
        return Err(b"dma-ndtr");
    }
    // DIR=10 (mem2mem) at bits 7:6, MINC (10) + PINC (9), PSIZE/MSIZE = 32-bit
    // (bits 12:11 and 14:13 = 0b10), then EN (bit 0).
    let cr = (0b10 << 6) | (1 << 10) | (1 << 9) | (0b10 << 11) | (0b10 << 13);
    wr32(s0cr, cr);
    wr32(s0cr, cr | 1); // EN

    let mut tc = false;
    for _ in 0..50_000 {
        if rd32(lisr) & DMA_TCIF0 != 0 {
            tc = true;
            break;
        }
    }
    if !tc {
        return Err(b"dma-tcif");
    }
    if rd32(s0ndtr) != 0 {
        return Err(b"dma-ndtr-drain");
    }
    // The flag is not the claim — the moved bytes are.
    #[allow(static_mut_refs)]
    unsafe {
        for i in 0..4usize {
            let want = read_volatile(core::ptr::addr_of!(DMA_SRC[i]));
            let got = read_volatile(core::ptr::addr_of!(DMA_DST[i]));
            if want != got {
                return Err(b"dma-payload");
            }
        }
    }
    wr32(lifcr, 0x3D); // W1C the stream-0 flags
    Ok(())
}

/// Returns the addresses of the static DMA source/destination buffers so a
/// fixture can pass them into [`check_dma`] without duplicating the storage.
pub fn dma_buffers() -> (u32, u32) {
    (
        core::ptr::addr_of!(DMA_SRC) as u32,
        core::ptr::addr_of!(DMA_DST) as u32,
    )
}

/// irq: NVIC set-pending / clear-pending round-trip on the EXTI0 vector
/// (position 6 on F4, RM0368 §11.1.3). PRIMASK is set for the duration of
/// this check via `cpsid i`/`cpsie i`: cortex-m-rt's default reset state
/// leaves interrupts globally enabled, and this fixture defines no EXTI0
/// handler, so an unmasked ISER+ISPR combination genuinely vectors to
/// cortex-m-rt's `DefaultHandler` (an infinite loop) the instant it's set —
/// the simulator faithfully takes the pended, enabled interrupt like real
/// silicon would. Masking with PRIMASK proves the NVIC's pending and enable
/// register banks track writes without depending on a handler existing.
/// EXTI's IMR/SWIER path is exercised alongside: a software-trigger write
/// must latch the pending-request register.
pub fn check_irq(exti_base: u32, exti0_irq: u32) -> Result<(), &'static [u8]> {
    const NVIC_ISER0: u32 = 0xE000_E100; // NVIC set-enable, IRQ 0..31
    const NVIC_ISPR0: u32 = 0xE000_E200; // NVIC set-pending, IRQ 0..31
    const NVIC_ICPR0: u32 = 0xE000_E280; // NVIC clear-pending, IRQ 0..31

    // SAFETY: `cpsid i` masks all exceptions below NMI/HardFault priority
    // (PRIMASK=1); nothing about it invalidates Rust's memory model. Always
    // paired with `cpsie i` below, including on every early-return path.
    unsafe { core::arch::asm!("cpsid i") };
    let result = check_irq_masked(exti_base, exti0_irq, NVIC_ISER0, NVIC_ISPR0, NVIC_ICPR0);
    unsafe { core::arch::asm!("cpsie i") };
    result
}

fn check_irq_masked(
    exti_base: u32,
    exti0_irq: u32,
    nvic_iser0: u32,
    nvic_ispr0: u32,
    nvic_icpr0: u32,
) -> Result<(), &'static [u8]> {
    // NVIC enable bank round-trip.
    wr32(nvic_iser0, 1 << exti0_irq);
    if rd32(nvic_iser0) & (1 << exti0_irq) == 0 {
        return Err(b"irq-iser");
    }
    // Pending bank: set, observe, clear, observe.
    wr32(nvic_icpr0, 1 << exti0_irq); // start from a known-clear state
    wr32(nvic_ispr0, 1 << exti0_irq);
    if rd32(nvic_ispr0) & (1 << exti0_irq) == 0 {
        return Err(b"irq-ispr");
    }
    wr32(nvic_icpr0, 1 << exti0_irq);
    if rd32(nvic_ispr0) & (1 << exti0_irq) != 0 {
        return Err(b"irq-icpr");
    }
    // EXTI software interrupt: IMR @ 0x00 unmask line 0, SWIER @ 0x10 trigger,
    // PR @ 0x14 must latch, and a W1C to PR must clear it (RM0368 §10.3).
    wr32(exti_base, rd32(exti_base) | 1); // IMR line 0
    wr32(exti_base + 0x10, 1); // SWIER line 0
    if rd32(exti_base + 0x14) & 1 == 0 {
        return Err(b"irq-exti-pr");
    }
    wr32(exti_base + 0x14, 1); // W1C
    if rd32(exti_base + 0x14) & 1 != 0 {
        return Err(b"irq-exti-clear");
    }
    // EXTI0's SWIER trigger genuinely asserts the wired hardware IRQ6 request,
    // latching NVIC's pending bit independently of the earlier software
    // ICPR/ISPR poke — clearing EXTI's own PR does not retract an
    // already-latched NVIC pend. Leave the NVIC clean (disabled, unpended)
    // before PRIMASK is lifted in the caller, or the vector fires the instant
    // interrupts are re-enabled.
    wr32(nvic_icpr0, 1 << exti0_irq);
    const NVIC_ICER0: u32 = 0xE000_E180; // NVIC clear-enable, IRQ 0..31
    wr32(NVIC_ICER0, 1 << exti0_irq);
    Ok(())
}

/// pwm: TIM1 advanced-control timer, gated on RCC_APB2ENR.TIM1EN (bit 0).
/// While the gate is off an ARR write is dropped. After enabling: program
/// PWM mode 1 on channel 1 (CCMR1.OC1M = 0b110, bits 6:4), enable the output
/// (CCER.CC1E) and the main output (BDTR.MOE — the advanced-timer-only gate
/// without which no PWM reaches the pin, RM0368 §12.4.18), then confirm the
/// counter runs and the compare register round-trips.
pub fn check_pwm(rcc_apb2enr: u32, tim1_base: u32, tim1en_bit: u32) -> Result<(), &'static [u8]> {
    // Gate OFF out of reset → write dropped, register reads back 0.
    wr32(tim1_base + 0x2C, 0xFFFF); // ARR while unclocked
    if rd32(tim1_base + 0x2C) != 0 {
        return Err(b"pwm-gated");
    }
    wr32(rcc_apb2enr, rd32(rcc_apb2enr) | (1 << tim1en_bit)); // TIM1EN

    wr32(tim1_base + 0x2C, 999); // ARR @ 0x2C: period
    if rd32(tim1_base + 0x2C) != 999 {
        return Err(b"pwm-arr");
    }
    wr32(tim1_base + 0x34, 250); // CCR1 @ 0x34: 25% duty
    if rd32(tim1_base + 0x34) != 250 {
        return Err(b"pwm-ccr1");
    }
    wr32(tim1_base + 0x18, 0b110 << 4); // CCMR1 @ 0x18: OC1M = PWM mode 1
    wr32(tim1_base + 0x20, 1); // CCER @ 0x20: CC1E
    wr32(tim1_base + 0x44, 1 << 15); // BDTR @ 0x44: MOE
    if rd32(tim1_base + 0x44) & (1 << 15) == 0 {
        return Err(b"pwm-moe");
    }
    wr32(tim1_base + 0x14, 0x01); // EGR @ 0x14: UG — latch the preloaded ARR/CCR
    wr32(tim1_base + 0x24, 0); // CNT @ 0x24
    wr32(tim1_base, 1); // CR1 @ 0x00: CEN
    let mut advanced = false;
    for _ in 0..50_000 {
        if rd32(tim1_base + 0x24) != 0 {
            advanced = true;
            break;
        }
    }
    if !advanced {
        return Err(b"pwm-cnt");
    }
    wr32(tim1_base, 0); // CEN off
    Ok(())
}
