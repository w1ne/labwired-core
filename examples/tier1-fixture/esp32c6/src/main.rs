//! ESP32-C6 Tier-1 fixture firmware.
//!
//! Validates the simulator's ESP32-C6 chip model peripheral-by-peripheral with
//! RAW REGISTER accesses and reports one line per peripheral class over UART0
//! using the TIER1 protocol:
//!
//! ```text
//! TIER1 <class> PASS
//! TIER1 <class> FAIL code=<reason>
//! TIER1 done
//! ```
//!
//! The `uart` class is implicit: receiving `TIER1 done` over UART0 proves UART.
//!
//! # Peripheral coverage against esp32c6.yaml
//!
//! | YAML id           | CLASS_MARKER  | class  | status  |
//! |-------------------|---------------|--------|---------|
//! | uart0             | uart          | uart   | implicit PASS via `done` |
//! | gpio              | gpio          | gpio   | PASS — OUT/ENABLE W1TS+W1TC side effects, matrix selectors, IN reads back the driven pad |
//! | interrupt_core0   | interrupt     | irq    | PASS — real CPU traps from the software doorbell AND from UART0's peripheral source |
//! | pcr               | pcr           | clock  | PASS — PCR register round-trip + CLK_EN=0 really kills UART0 MMIO |
//! | timg0             | timg          | timer  | PASS — T0UPDATE latches an advancing counter; disabled T0 frozen |
//! | gdma              | gdma          | dma    | PASS — descriptor-driven mem→mem copy + status flags + owner writeback |
//! | i2c0              | i2c           | i2c    | PASS — C3 command-list engine at the C6 base: TRANS_COMPLETE + COMD done |
//! | spi2              | spi           | spi    | PASS — GP-SPI2 USR handshake + TRANS_DONE + idle-MISO data |
//! | apb_saradc        | adc           | adc    | PASS — one-shot channel-dependent conversion + DONE handshake |
//! | ledc              | ledc          | pwm    | PASS — live LEDC timer advances, wraps (LSTIMER0_OVF), PAUSE freezes |
//! | timg0 / timg1     | esp32c6_mwdt  | wdt    | PASS — WDTWPROTECT write lock + CONFIG round-trip + full stage-0→1 chain latch |
//! | lp_timer          | esp32c6_lp_rtc| rtc    | PASS — LP_TIMER 48-bit counter snapshot advance + BUF0→BUF1 shift |
//!
//! The `wdt` class is served by the TIMG MWDT inside `timg0`/`timg1` (type
//! `esp32c6_mwdt`); the LP_WDT window is NOT declared. The `rtc` class is the
//! C6 LP_TIMER model, not the C3 RTC_CNTL timer (different register shape).
//!
//! What is still NOT declared (SYSTIMER, TWAI, USB-Serial/JTAG, radios, the LP
//! core, LP_WDT, LP_I2C/LP_UART) has no fixture claim; the harness renders
//! those classes `na`. The per-class claim boundaries ("what is proven" and
//! "what is NOT claimed") are stated on each check below and in
//! docs/boards/esp32c6-devkitc.md.
//!
//! # clock: what the PCR check proves — and does not
//!
//! `pcr` is a native register file (the full SVD PCR map) and is the clock
//! controller the chip yaml's `clock:` gates resolve through. The check
//! (1) round-trips `UART0_SCLK_CONF` and `SYSCLK_CONF` (real clock-source
//! registers, reset 0x2800_0200 for SYSCLK_CONF), and (2) closes
//! `UART0_CONF.CLK_EN`, observes that a UART0 register read returns 0 and
//! that a write while gated is DROPPED (the pre-gate value survives), then
//! reopens the gate and observes the register live again. `RST_EN` is
//! recorded but not enforced — the fixture does not claim reset semantics.
//!
//! # timer: what the TIMG check proves — and does not
//!
//! The C6 TIMG0 is at 0x6000_8000 (`esp32c6_mwdt` — the shared
//! `esp32::timg::Timg` model with the C3/C6 MWDT path armed). The check writes
//! `T0CONFIG` (EN|INCREASE, divider 1), latches `T0LO/T0HI` twice around a
//! bounded spin and requires the value to advance; it then clears `EN`, latches
//! twice more and requires the counter to be frozen. Peripherals are clocked at
//! reset (`TIMERGROUP0_CONF.CLK_EN` = 1); the check does not claim alarm/IRQ
//! delivery. The watchdog has its own `wdt` check.
//!
//! # dma: what the GDMA check proves — and does not
//!
//! The C6 GDMA is at 0x6008_0000 (`esp32c6_gdma`, 3 channels, C6 register
//! layout). The check builds a real in-RAM linked-list pair (TX descriptor →
//! source buffer, RX descriptor → destination buffer), sets
//! `IN_CONF0.MEM_TRANS_EN`, binds both PERI_SELs to Dummy-1 (TRM §3.4.3),
//! starts both links, polls `IN_INT_RAW` for `IN_SUC_EOF`, then verifies the
//! bytes actually landed in the destination, the DONE/TOTAL_EOF flags
//! latched, and the owner bits were returned to the CPU (`OUT_AUTO_WRBACK`).
//! It does NOT claim peripheral-coupled DMA (SPI/UART/I2S/AES/...): that
//! datapath is unimplemented and stalls visibly in the model.
//!
//! # irq: real interrupt delivery, not a register round-trip
//!
//! The C6 splits the interrupt fabric across two blocks: source MAP registers in
//! `INTERRUPT_CORE0` @0x6001_0000 (SVD offsets src*4) and the
//! enable/priority/threshold gates plus the `CPU_INTR_FROM_CPU_n` doorbells in
//! `INTPRI` @0x600C_5000. The engine routes asserted sources through those
//! gates into the RISC-V core's external lines
//! (`crates/core/src/bus/routing.rs`, C6 INTPRI layout arm).
//!
//! This check proves the full path end to end for TWO sources. It installs its
//! own `mtvec` trap entry, maps doorbell source 22 to CPU line 9, enables line
//! 9 with a passing priority, sets `mstatus.MIE`, rings the doorbell, and
//! requires the handler to RUN with `mcause = 0x8000_0009` (machine external
//! interrupt on line 9) and to acknowledge the doorbell. It then proves the
//! enable gate is real by disabling line 9 and ringing again without a second
//! trap. A declarative register file cannot produce a trap.
//!
//! The second source is a real PERIPHERAL, not a software doorbell: UART0's
//! `irq: 43` wiring makes the shared Espressif twin assert matrix source 43,
//! which this check maps to line 10. It programs `INT_ENA.TX_DONE`, pushes one
//! byte through the TX FIFO so the latched `TX_DONE` raw bit rises on the
//! drain, and requires a trap with `mcause = 0x8000_000A`; the handler's
//! `UART_INT_CLR` W1C write clears the source (and masks the line, because the
//! model re-derives a scheduler-driven level at the peripheral tick — see the
//! handler), then the main flow RE-ENABLES line 10 with MIE on and requires no
//! second trap, which is what proves the source really de-asserted. (The
//! software doorbell cannot prove the peripheral path: it is a separate
//! INTPRI register, not a peripheral `matrix_irq_sources_into`.)
//!
//! Register offsets follow the ESP32-C6 SVD (`tests/fixtures/real_world/
//! esp32c6.svd`) and are cross-checked against the simulator's declarative
//! models in `configs/peripherals/esp32c6/`.

#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

// ── Peripheral base addresses (esp-idf v5.3 reg_base.h) ───────────────────
const UART0_BASE: u32 = 0x6000_0000;
const I2C0_BASE: u32 = 0x6000_4000;
const LEDC_BASE: u32 = 0x6000_7000;
const TIMG0_BASE: u32 = 0x6000_8000;
const APB_SARADC_BASE: u32 = 0x6000_E000;
const INTERRUPT_CORE0_BASE: u32 = 0x6001_0000;
const GDMA_BASE: u32 = 0x6008_0000;
const SPI2_BASE: u32 = 0x6008_1000;
const PCR_BASE: u32 = 0x6009_6000;
const GPIO_BASE: u32 = 0x6009_1000;
const LP_TIMER_BASE: u32 = 0x600B_0C00;
const INTPRI_BASE: u32 = 0x600C_5000;

/// INTPRI control registers (C6 layout): `CPU_INT_ENABLE` line mask and the
/// `CPU_INT_PRI_n` / `CPU_INT_THRESH` fields used by the irq check.
const CPU_INT_ENABLE: u32 = INTPRI_BASE + 0x00;
const CPU_INT_PRI_BASE: u32 = INTPRI_BASE + 0x0C;
const CPU_INT_THRESH: u32 = INTPRI_BASE + 0x8C;

/// `CPU_INTR_FROM_CPU_0` matrix source (`ETS_FROM_CPU_INTR0_SOURCE`); the C3
/// numbers the same doorbell 50. `MAP` word @ INTERRUPT_CORE0 + 22*4.
const SOURCE_FROM_CPU_0: u32 = 22;
/// `UART0` matrix source (`ETS_UART0_INTR_SOURCE`, esp32c6.svd); the C3 numbers
/// the same console UART 21. `MAP` word @ INTERRUPT_CORE0 + 43*4.
const SOURCE_UART0: u32 = 43;
/// `CPU_INTR_FROM_CPU_0` doorbell register inside INTPRI.
const CPU_INTR_FROM_CPU_0: u32 = INTPRI_BASE + 0x90;
/// CPU interrupt line the doorbell is routed to (1..31).
const IRQ_LINE: u32 = 9;
/// `mcause` for a machine external interrupt (`0x8000_0000 | line`).
const EXPECT_CAUSE: u32 = 0x8000_0000 | IRQ_LINE;
/// CPU line the UART0 peripheral source is routed to (distinct from the
/// doorbell's 9 so the two proofs cannot alias).
const UART0_IRQ_LINE: u32 = 10;
/// `mcause` the UART0-sourced trap must carry.
const EXPECT_CAUSE_UART0: u32 = 0x8000_0000 | UART0_IRQ_LINE;

// UART0 interrupt registers (shared Espressif twin).
const UART0_INT_RAW: u32 = UART0_BASE + 0x04;
const UART0_INT_ENA: u32 = UART0_BASE + 0x0C;
const UART0_INT_CLR: u32 = UART0_BASE + 0x10;
/// `UART_INTR_TX_DONE` (`uart_ll.h` bit 14): the LATCHED edge bit set when
/// the TX FIFO empties, W1C via INT_CLR. A latched edge is what makes the
/// handler's `UART_INT_CLR` a true source de-assert (unlike the live
/// `TXFIFO_EMPTY` level, which INT_CLR cannot clear).
const UART_INT_TX_DONE: u32 = 1 << 14;

#[inline(always)]
fn rd32(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
fn wr32(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

// ── UART0 raw byte output ─────────────────────────────────────────────────
//
// UART0 (0x6000_0000): FIFO @ 0x00 (32-bit write pushes the low byte), STATUS
// @ 0x1C with TXFIFO_CNT in bits [25:16]. The C6 binds the shared Espressif
// UART twin: a 128-entry TX FIFO that drops a byte written while full, so poll
// for space exactly as driver firmware does (same shape as the C3 fixture).
fn uart0_write_byte(byte: u8) {
    const FIFO: u32 = UART0_BASE;
    const STATUS: u32 = UART0_BASE + 0x1C;
    const TXFIFO_LEN: u32 = 128; // SOC_UART_FIFO_LEN

    for _ in 0..1_000_000 {
        if ((rd32(STATUS) >> 16) & 0x3FF) < TXFIFO_LEN {
            break;
        }
    }
    // On timeout, write anyway; a garbled line beats a hung fixture.
    wr32(FIFO, byte as u32);
}

fn uart0_write_str(s: &str) {
    for b in s.as_bytes() {
        uart0_write_byte(*b);
    }
}

fn uart0_write_line(s: &str) {
    uart0_write_str(s);
    uart0_write_str("\r\n");
}

fn report(class: &str, result: Result<(), &'static str>) {
    uart0_write_str("TIER1 ");
    uart0_write_str(class);
    match result {
        Ok(()) => uart0_write_line(" PASS"),
        Err(code) => {
            uart0_write_str(" FAIL code=");
            uart0_write_line(code);
        }
    }
}

// ── gpio: output enable + W1TS/W1TC side effects + matrix words ────────────
//
// configs/chips/esp32c6.yaml wires `gpio` (type esp32c6_gpio) to the C3 GPIO
// model (offset-identical register head): OUT @0x04, OUT_W1TS @0x08,
// OUT_W1TC @0x0C, ENABLE @0x20 (+W1TS 0x24/W1TC 0x28), IN @0x3C,
// FUNC0_IN_SEL_CFG @0x154, FUNC0_OUT_SEL_CFG @0x554. Pins used are GPIO4/GPIO5
// — inside the modelled 26-pin block and not boot straps.
//
// What is proven: the set/clear aliases are NOT independent storage on this
// model — OUT_W1TS/OUT_W1TC really mutate the OUT latch and ENABLE_W1TS/W1TC
// really mutate ENABLE, read back through OUT/ENABLE. The pad-selector words
// of both matrices retain what firmware writes. IN is the pad-input word: a
// pad whose output driver is on samples itself through its IO_MUX input
// buffer, so IN READS BACK the driven level while the latch is high and drops
// it again when the latch clears.
//
// What is NOT claimed: IO_MUX pad routing is declarative on the C6 (the yaml
// says so), so this does not prove electrical pad behaviour, and no external
// device drives these pins here.
fn check_gpio() -> Result<(), &'static str> {
    const OUT: u32 = GPIO_BASE + 0x04;
    const OUT_W1TS: u32 = GPIO_BASE + 0x08;
    const OUT_W1TC: u32 = GPIO_BASE + 0x0C;
    const ENABLE: u32 = GPIO_BASE + 0x20;
    const ENABLE_W1TS: u32 = GPIO_BASE + 0x24;
    const ENABLE_W1TC: u32 = GPIO_BASE + 0x28;
    const IN: u32 = GPIO_BASE + 0x3C;
    const FUNC0_IN_SEL_CFG: u32 = GPIO_BASE + 0x154;
    const FUNC0_OUT_SEL_CFG: u32 = GPIO_BASE + 0x554;
    const P4: u32 = 1 << 4;
    const P5: u32 = 1 << 5;
    // SIG_GPIO_OUT — pad driven by the plain GPIO_OUT latch.
    const SIG_GPIO_OUT: u32 = 128;
    // C6 matrix output signal index used by the UART0 transmitter
    // (esp-idf v5.3 `gpio_sig_map.h` for esp32c6: `U0TXD_OUT_IDX`).
    const SIG_U0TXD: u32 = 6;

    // ENABLE: plain store, then the W1TS/W1TC aliases must set/clear bits.
    wr32(ENABLE, P4);
    if rd32(ENABLE) & P4 == 0 {
        return Err("gpio-enable-store");
    }
    wr32(ENABLE_W1TS, P5);
    if rd32(ENABLE) & (P4 | P5) != (P4 | P5) {
        return Err("gpio-enable-w1ts");
    }
    wr32(ENABLE_W1TC, P4);
    if rd32(ENABLE) & (P4 | P5) != P5 {
        return Err("gpio-enable-w1tc");
    }

    // OUT: plain store, then W1TS set / W1TC clear read back through OUT.
    wr32(OUT, P4);
    if rd32(OUT) & P4 == 0 {
        return Err("gpio-out-store");
    }
    wr32(OUT_W1TS, P5);
    if rd32(OUT) & (P4 | P5) != (P4 | P5) {
        return Err("gpio-out-w1ts");
    }
    wr32(OUT_W1TC, P4);
    if rd32(OUT) & (P4 | P5) != P5 {
        return Err("gpio-out-w1tc");
    }

    // GPIO matrix: the output selector for pad 4 (FUNC4_OUT_SEL_CFG) and one
    // input-matrix word (FUNC6_IN_SEL_CFG) must round-trip.
    wr32(FUNC0_OUT_SEL_CFG + 4 * 4, SIG_GPIO_OUT);
    if rd32(FUNC0_OUT_SEL_CFG + 4 * 4) & 0x1FF != SIG_GPIO_OUT {
        return Err("gpio-matrix-out-sel");
    }
    wr32(FUNC0_OUT_SEL_CFG + 4 * 4, SIG_U0TXD);
    if rd32(FUNC0_OUT_SEL_CFG + 4 * 4) & 0x1FF != SIG_U0TXD {
        return Err("gpio-matrix-out-sel-rewrite");
    }
    // Input selector: MATRIX_INPUT_SELECT (bit 6) | source pad 3.
    let in_sel_word: u32 = (1 << 6) | 3;
    wr32(FUNC0_IN_SEL_CFG + 6 * 4, in_sel_word);
    if rd32(FUNC0_IN_SEL_CFG + 6 * 4) != in_sel_word {
        return Err("gpio-matrix-in-sel");
    }

    // IN reads the pad back through its input buffer while the output driver
    // is on (model behaviour since #1344). GPIO4/GPIO5 leave reset with
    // IO_MUX FUN_IE=1: ESP32-C6 TRM v1.2 Table 7.12-1 gives both pads
    // Reset=1 ("IE = 1"), and datasheet v1.5 Tables 2-1/2-2 list MTMS/MTDI as
    // IE at and after reset. (The SVD's uniform 0x800 IO_MUX reset word, which
    // says FUN_IE=0, disagrees with both and is wrong for these pads.) So an
    // enabled output pad samples the level it drives, and IN must FOLLOW OUT
    // here, and drop again when the latch clears.
    //
    // Pad 4's output selector was rewritten to SIG_U0TXD above (that is the
    // matrix round-trip test); hand it back to the plain GPIO_OUT latch first,
    // or the pad is driven by the UART signal and not by OUT.
    wr32(FUNC0_OUT_SEL_CFG + 4 * 4, SIG_GPIO_OUT);
    wr32(ENABLE, P4 | P5);
    wr32(OUT, P4 | P5);
    if rd32(IN) & (P4 | P5) != (P4 | P5) {
        return Err("gpio-in-follows-out");
    }
    // Clearing the latch without touching ENABLE clears the readback too —
    // this is the driven level, not a latched stale input.
    wr32(OUT, 0);
    if rd32(IN) & (P4 | P5) != 0 {
        return Err("gpio-in-latch-clear");
    }
    // Driving one pad high and the other low must read back exactly that
    // pattern, not an all-or-nothing word.
    wr32(OUT, P4);
    if rd32(IN) & (P4 | P5) != P4 {
        return Err("gpio-in-mixed");
    }

    // Leave the block quiet for the rest of the run.
    wr32(OUT, 0);
    wr32(ENABLE, 0);
    Ok(())
}

// ── irq: real traps from a software AND a peripheral source ────────────────
//
// ESP32-C6 matrix layout (crates/core/src/bus/routing.rs, C6 arm):
//   * source MAP: INTERRUPT_CORE0 @0x6001_0000 + source*4, low 5 bits = line;
//   * CPU_INT_ENABLE @ INTPRI+0x00, CPU_INT_PRI_n @ INTPRI+0x0C+n*4,
//     CPU_INT_THRESH @ INTPRI+0x8C;
//   * CPU_INTR_FROM_CPU_0 doorbell @ INTPRI+0x90 (matrix source 22).
// The RISC-V core takes a machine external interrupt at the end of the store
// instruction whose write choke re-routes the asserted source (or at the next
// peripheral tick), landing on `mtvec` with `mcause = 0x8000_0000 | line`.
//
// Two sources are proven, on two different CPU lines:
//   1. the software doorbell (source 22 -> line 9), which also carries the
//      enable-gate proof (line 9 disabled => no second trap);
//   2. UART0 (source 43 -> line 10): `INT_ENA.TX_DONE` is armed, one byte is
//      shifted out of the TX FIFO so the latched TX_DONE raw bit rises, the
//      handler's `UART_INT_CLR` W1C ack drops the source (and masks the line
//      because the model re-derives a scheduler-driven level at the tick),
//      then the main flow RE-ENABLES the line and requires no further trap.
//      This is a REAL peripheral source, not the software doorbell: the UART
//      itself asserts source 43 through its `irq: 43` descriptor wiring and
//      the shared twin's `matrix_irq_sources_into`.

/// Trap handler state, touched by the assembly entry through `trap_dispatch`.
static mut TRAP_COUNT: u32 = 0;
static mut TRAP_CAUSE: u32 = 0;

core::arch::global_asm!(
    r#"
    .section .text.trap_entry
    .align 4
    .global trap_entry
trap_entry:
    addi sp, sp, -128
    sw ra,   0(sp)
    sw t0,   4(sp)
    sw t1,   8(sp)
    sw t2,  12(sp)
    sw t3,  16(sp)
    sw t4,  20(sp)
    sw t5,  24(sp)
    sw t6,  28(sp)
    sw a0,  32(sp)
    sw a1,  36(sp)
    sw a2,  40(sp)
    sw a3,  44(sp)
    sw a4,  48(sp)
    sw a5,  52(sp)
    sw a6,  56(sp)
    sw a7,  60(sp)
    call trap_dispatch
    lw ra,   0(sp)
    lw t0,   4(sp)
    lw t1,   8(sp)
    lw t2,  12(sp)
    lw t3,  16(sp)
    lw t4,  20(sp)
    lw t5,  24(sp)
    lw t6,  28(sp)
    lw a0,  32(sp)
    lw a1,  36(sp)
    lw a2,  40(sp)
    lw a3,  44(sp)
    lw a4,  48(sp)
    lw a5,  52(sp)
    lw a6,  56(sp)
    lw a7,  60(sp)
    addi sp, sp, 128
    mret
"#
);

extern "C" {
    fn trap_entry();
}

/// Rust body of the trap entry. Records why the trap happened and
/// acknowledges the source so its level de-asserts: the software doorbell for
/// line 9, `UART_INT_CLR` (W1C `TX_DONE`) for the UART0 line.
#[no_mangle]
pub extern "C" fn trap_dispatch() {
    let cause: u32;
    unsafe { core::arch::asm!("csrr {}, mcause", out(reg) cause) };
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_CAUSE), cause);
        let count = core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT));
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_COUNT), count.wrapping_add(1));
    }
    if cause == EXPECT_CAUSE_UART0 {
        // TX_DONE is a latched edge: W1C clears it and the source level drops.
        wr32(UART0_INT_CLR, UART_INT_TX_DONE);
        // The bus re-derives a scheduler-driven peripheral's matrix level at
        // the peripheral tick, not at this MMIO write, so an unmasked
        // level-triggered line would re-enter this handler until the next
        // tick. Mask the line (an INTPRI write, recomputed at the store);
        // check_irq re-enables it and requires no further trap — which is
        // what proves the INT_CLR ack actually de-asserted the source.
        wr32(CPU_INT_ENABLE, 0);
    } else {
        // CPU_INTR_FROM_CPU_0 is a level bit: writing 0 clears the pending
        // source.
        wr32(CPU_INTR_FROM_CPU_0, 0);
    }
}

#[inline(always)]
fn mstatus_mie_enable() {
    unsafe { core::arch::asm!("csrs mstatus, {0}", in(reg) 8u32) };
}

#[inline(always)]
fn mstatus_mie_disable() {
    unsafe { core::arch::asm!("csrc mstatus, {0}", in(reg) 8u32) };
}

fn check_irq() -> Result<(), &'static str> {
    const MAP_FROM_CPU0: u32 = INTERRUPT_CORE0_BASE + SOURCE_FROM_CPU_0 * 4;

    // The MAP word itself must be a real register first (the C3-tier check
    // still holds on the C6): write, read back, overwrite, read back.
    wr32(MAP_FROM_CPU0, IRQ_LINE);
    if rd32(MAP_FROM_CPU0) & 0x1F != IRQ_LINE {
        return Err("intmatrix-map-readback");
    }
    wr32(MAP_FROM_CPU0, 5);
    if rd32(MAP_FROM_CPU0) & 0x1F != 5 {
        return Err("intmatrix-map-rewrite");
    }
    wr32(MAP_FROM_CPU0, IRQ_LINE);

    // Install our trap entry and program the C6 gates: line 9 enabled with a
    // priority above the reset threshold.
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_COUNT), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_CAUSE), 0);
        core::arch::asm!("csrw mtvec, {0}", in(reg) trap_entry as *const () as usize);
    }
    wr32(CPU_INT_ENABLE, 1 << IRQ_LINE);
    wr32(CPU_INT_PRI_BASE + IRQ_LINE * 4, 1);
    wr32(CPU_INT_THRESH, 0);

    if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)) } != 0 {
        return Err("irq-trap-early");
    }

    // Ring the doorbell with machine interrupts enabled. The write choke
    // recomputes the routed line on the store, so the trap lands immediately;
    // the bounded poll tolerates a tick-quantised delivery.
    mstatus_mie_enable();
    wr32(CPU_INTR_FROM_CPU_0, 1);
    let mut delivered = false;
    for _ in 0..1_000_000 {
        if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)) } != 0 {
            delivered = true;
            break;
        }
    }
    mstatus_mie_disable();
    wr32(CPU_INTR_FROM_CPU_0, 0);

    if !delivered {
        return Err("irq-not-delivered");
    }
    if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)) } != 1 {
        return Err("irq-trap-count");
    }
    if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_CAUSE)) } != EXPECT_CAUSE {
        return Err("irq-trap-cause");
    }

    // The enable gate is real: with line 9 disabled the same doorbell must NOT
    // trap. (This also proves the first trap's doorbell was acknowledged.)
    wr32(CPU_INT_ENABLE, 0);
    mstatus_mie_enable();
    wr32(CPU_INTR_FROM_CPU_0, 1);
    for i in 0u32..20_000 {
        core::hint::black_box(i);
    }
    let traps_while_masked = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)) };
    wr32(CPU_INTR_FROM_CPU_0, 0);
    mstatus_mie_disable();
    if traps_while_masked != 1 {
        return Err("irq-enable-gate");
    }

    // ── 2. Peripheral source: UART0 (43) -> line 10 ────────────────────────
    //
    // The UART0 MAP word must be a real register (same proof the doorbell got).
    const MAP_UART0: u32 = INTERRUPT_CORE0_BASE + SOURCE_UART0 * 4;
    wr32(MAP_UART0, UART0_IRQ_LINE);
    if rd32(MAP_UART0) & 0x1F != UART0_IRQ_LINE {
        return Err("uart-intmatrix-map-readback");
    }

    // Wait for the console FIFO to finish draining, then clear every stale
    // raw bit — the fixture has been printing over UART0, so TX_DONE is
    // almost certainly already latched. `TXFIFO_CNT == 0` also means the
    // model has applied the drain's TX_DONE edge, so the clear below is the
    // last word before the deliberate transfer.
    for _ in 0..2_000_000 {
        if (rd32(UART0_BASE + 0x1C) >> 16) & 0x3FF == 0 {
            break;
        }
    }
    wr32(UART0_INT_CLR, 0xFFFF_FFFF);
    if rd32(UART0_INT_RAW) & UART_INT_TX_DONE != 0 {
        return Err("uart-tx-done-not-cleared");
    }

    // Enable line 10 with a passing priority, and arm ONLY TX_DONE (a latched
    // edge, W1C-clearable) so the handler's ack is a true de-assert.
    wr32(CPU_INT_ENABLE, 1 << UART0_IRQ_LINE);
    wr32(CPU_INT_PRI_BASE + UART0_IRQ_LINE * 4, 1);
    wr32(CPU_INT_THRESH, 0);
    wr32(UART0_INT_ENA, UART_INT_TX_DONE);
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_COUNT), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_CAUSE), 0);
    }

    // Push one byte into the TX FIFO. It shifts out at the configured baud
    // rate; when the FIFO empties, TX_DONE latches, source 43 asserts, and
    // the line 10 gate delivers the trap. The byte echoes to the console —
    // harmless between TIER1 lines.
    mstatus_mie_enable();
    wr32(UART0_BASE, b'\r' as u32);
    let mut uart_delivered = false;
    for _ in 0..1_000_000 {
        if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)) } != 0 {
            uart_delivered = true;
            break;
        }
    }
    mstatus_mie_disable();

    if !uart_delivered {
        return Err("uart-irq-not-delivered");
    }
    if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)) } != 1 {
        return Err("uart-irq-trap-count");
    }
    if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_CAUSE)) } != EXPECT_CAUSE_UART0 {
        return Err("uart-irq-trap-cause");
    }
    // The handler ran before mret, so its W1C ack is already visible here.
    if rd32(UART0_INT_RAW) & UART_INT_TX_DONE != 0 {
        return Err("uart-intclr-not-w1c");
    }

    // Give the bus at least one peripheral tick with interrupts masked, so
    // the scheduler-driven UART level is re-derived from the cleared
    // INT_RAW (the model's documented tick-quantised de-assert bound).
    for i in 0u32..20_000 {
        core::hint::black_box(i);
    }

    // De-assert proof: re-enable line 10 and MIE. If the source were still
    // pending — or INT_CLR had not really cleared it — the re-enabled line
    // would trap again. No second trap = the line really dropped.
    wr32(CPU_INT_ENABLE, 1 << UART0_IRQ_LINE);
    mstatus_mie_enable();
    for i in 0u32..20_000 {
        core::hint::black_box(i);
    }
    let uart_traps_after_ack = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)) };
    mstatus_mie_disable();
    if uart_traps_after_ack != 1 {
        return Err("uart-line-not-deasserted");
    }

    // Leave the matrix and the UART interrupt quiet for the rest of the run.
    wr32(CPU_INT_ENABLE, 0);
    wr32(MAP_FROM_CPU0, 0);
    wr32(MAP_UART0, 0);
    wr32(UART0_INT_ENA, 0);
    wr32(UART0_INT_CLR, 0xFFFF_FFFF);
    Ok(())
}

// ── clock: PCR register round-trip + enforced UART0 clock gate ─────────────
//
// configs/chips/esp32c6.yaml declares `pcr` as `esp32c6_pcr` — a native
// register file (full SVD map) that also serves `clock_gate_reg_offset` for
// the chip yaml's `clock:` gates. The bus gate
// (crates/core/src/bus/device_hooks.rs::is_peripheral_clocked) reads the LIVE
// PCR register at every UART0 access: with CLK_EN clear, reads return 0 and
// writes are dropped; with it set, the console UART behaves normally.
//
// What is proven:
//   * UART0_SCLK_CONF (a clock-source/divider register) round-trips a value
//     that is not its reset, and SYSCLK_CONF reads its SVD reset value and
//     round-trips a write — PCR is a real register file, not read-as-zero.
//   * UART0_CONF.CLK_EN=0 makes a stored UART0 register read 0 (dead), and a
//     write issued while gated is DROPPED: after reopening the gate the old
//     value is still there. Then a normal read works again.
//
// What is NOT claimed: RST_EN semantics (recorded, not enforced), clock
// divisors/frequencies, and the SYSCLK mux. No silicon diff — the observed
// gating contract comes from the model + the SVD register map.
fn check_clock() -> Result<(), &'static str> {
    const UART0_CONF: u32 = PCR_BASE + 0x00;
    const UART0_SCLK_CONF: u32 = PCR_BASE + 0x04;
    const SYSCLK_CONF: u32 = PCR_BASE + 0x110;
    const CLK_EN: u32 = 1 << 0;

    // UART0_SCLK_CONF: SVD reset 0x0070_0000; write something else and read it.
    if rd32(UART0_SCLK_CONF) != 0x0070_0000 {
        return Err("pcr-sclk-reset-value");
    }
    wr32(UART0_SCLK_CONF, 0x0050_1234);
    if rd32(UART0_SCLK_CONF) != 0x0050_1234 {
        return Err("pcr-sclk-roundtrip");
    }
    wr32(UART0_SCLK_CONF, 0x0070_0000);

    // SYSCLK_CONF: SVD reset 0x2800_0200 (LS_DIV/HS_DIV); round-trip a change.
    if rd32(SYSCLK_CONF) != 0x2800_0200 {
        return Err("pcr-sysclk-reset-value");
    }
    wr32(SYSCLK_CONF, 0x2800_0202);
    if rd32(SYSCLK_CONF) != 0x2800_0202 {
        return Err("pcr-sysclk-roundtrip");
    }
    wr32(SYSCLK_CONF, 0x2800_0200);

    // Gate proof. CLK_EN is 1 at reset (the console runs), RST_EN is 0.
    if rd32(UART0_CONF) & CLK_EN == 0 {
        return Err("pcr-uart0-ungated-at-reset");
    }
    const UART0_CLKDIV: u32 = UART0_BASE + 0x14;
    const UART0_STATUS: u32 = UART0_BASE + 0x1C;

    // The gate experiment is a closure whose result is returned only AFTER
    // the gate has been reopened: a failure return while UART0 is dead would
    // hang the transcript instead of reporting it.
    let gate_probe = || -> Result<(), &'static str> {
        wr32(UART0_CLKDIV, 0x234);
        if rd32(UART0_CLKDIV) != 0x234 {
            return Err("clock-probe-store");
        }

        // Close the gate (CLK_EN=0): every UART0 register access is dead.
        wr32(UART0_CONF, 0);
        if rd32(UART0_CLKDIV) != 0 {
            return Err("clock-gated-read-not-dead");
        }
        if rd32(UART0_STATUS) != 0 {
            return Err("clock-gated-status-not-dead");
        }
        // A gated write must be dropped, not queued.
        wr32(UART0_CLKDIV, 0x0BAD_0BAD);

        // Reopen the gate (restore the SVD reset value): the peripheral
        // answers again, and the gated write is GONE — state, not a delayed
        // store.
        wr32(UART0_CONF, 1);
        if rd32(UART0_CLKDIV) != 0x234 {
            return Err("clock-gated-write-not-dropped");
        }
        if (rd32(UART0_STATUS) >> 16) & 0x3FF >= 128 {
            return Err("clock-ungated-status-dead");
        }
        Ok(())
    };
    let result = gate_probe();
    wr32(UART0_CONF, 1); // unconditional reopen before reporting
    result
}

// ── timer: timg0 counter advance + EN gate ─────────────────────────────────
//
// configs/chips/esp32c6.yaml wires timg0 at 0x6000_8000 as the shared
// `esp32::timg::Timg` model with the MWDT path armed (`esp32c6_mwdt`); the
// register head is offset-identical to the C3's, the C6 carries one
// general-purpose timer per group, and the model's T0 head is the part this
// check drives. T0CONFIG.EN (bit 31) gates the live counter; T0UPDATE latches
// the 64-bit value into T0LO/T0HI.
//
// What is proven: with EN set the latched value advances across a bounded
// spin, and with EN cleared the latched value is frozen. A register file
// cannot produce a counter that both runs and stops.
//
// What is NOT claimed: real-time rate (one count per model tick, not at the
// programmed divider), alarm interrupts, or watchdog behavior (that is
// check_wdt).
fn check_timer() -> Result<(), &'static str> {
    const T0CONFIG: u32 = TIMG0_BASE + 0x00;
    const T0LO: u32 = TIMG0_BASE + 0x04;
    const T0HI: u32 = TIMG0_BASE + 0x08;
    const T0UPDATE: u32 = TIMG0_BASE + 0x0C;
    const EN: u32 = 1 << 31;
    const INCREASE: u32 = 1 << 30;
    const DIVIDER_1: u32 = 1 << 13;

    // T0CONFIG is writable and the EN bit stores.
    wr32(T0CONFIG, EN | INCREASE | DIVIDER_1);
    if rd32(T0CONFIG) & EN == 0 {
        return Err("timg0-config-store");
    }

    let latch = || -> u64 {
        wr32(T0UPDATE, 1);
        ((rd32(T0HI) as u64) << 32) | rd32(T0LO) as u64
    };

    let a = latch();
    for i in 0u32..20_000 {
        core::hint::black_box(i);
    }
    let b = latch();
    if b <= a {
        return Err("timg0-not-counting");
    }

    // Clear EN: the counter must stop. (A second latch pair must be equal.)
    wr32(T0CONFIG, INCREASE | DIVIDER_1);
    let c = latch();
    for i in 0u32..20_000 {
        core::hint::black_box(i);
    }
    let d = latch();
    if d != c {
        return Err("timg0-counts-when-disabled");
    }
    Ok(())
}

// ── dma: GDMA mem→mem descriptor transfer ──────────────────────────────────
//
// esp32c6.yaml declares `gdma` (0x6008_0000, `esp32c6_gdma`, 3 channels).
// C6 register layout (SVD): per-channel IN block @0x70+n*0xC0 (IN_CONF0
// +0x00, IN_LINK +0x10, IN_PERI_SEL +0x30), OUT registers from 0xD4+n*0xC0
// (OUT_LINK +0x0C, OUT_PERI_SEL +0x2C) and OUT_CONF0 @0x190+n*0xC0; flat
// INT_* registers 0x00/0x30 + n*0x10. INLINK/OUTLINK carry the low 20 bits of
// the descriptor address; buffers and descriptors live in HP SRAM at
// 0x4080_0000.
//
// What is proven: a real linked-list copy. The fixture arms MEM_TRANS_EN and
// a Dummy-1 PERI_SEL on both directions (TRM §3.4.3), starts both links, and
// polls IN_INT_RAW. Completion requires the bytes to have actually landed in
// the destination buffer, the IN/OUT DONE+EOF flags, and the owner bits
// written back to the CPU (OUT_AUTO_WRBACK for TX; RX writeback always).
//
// What is NOT claimed: peripheral-coupled DMA (no pump exists in the model;
// such a start stalls visibly), FIFO data ports, priority arbitration, and
// interrupt delivery of the DMA sources.
#[repr(C)]
struct DmaDescriptor {
    dw0: u32,
    buffer: u32,
    next: u32,
}

static mut DMA_SRC: [u8; 16] = *b"TIER1-GDMA-M2M!\0";
static mut DMA_DST: [u8; 16] = [0u8; 16];
static mut DMA_TX_DESC: DmaDescriptor = DmaDescriptor {
    dw0: 0,
    buffer: 0,
    next: 0,
};
static mut DMA_RX_DESC: DmaDescriptor = DmaDescriptor {
    dw0: 0,
    buffer: 0,
    next: 0,
};

fn check_dma() -> Result<(), &'static str> {
    const IN_CONF0: u32 = GDMA_BASE + 0x70;
    const IN_INT_RAW: u32 = GDMA_BASE + 0x00;
    const IN_INT_CLR: u32 = GDMA_BASE + 0x0C;
    const IN_LINK: u32 = GDMA_BASE + 0x80;
    const IN_PERI_SEL: u32 = GDMA_BASE + 0xA0;
    const OUT_INT_RAW: u32 = GDMA_BASE + 0x30;
    const OUT_INT_CLR: u32 = GDMA_BASE + 0x3C;
    const OUT_LINK: u32 = GDMA_BASE + 0xE0;
    const OUT_PERI_SEL: u32 = GDMA_BASE + 0x100;
    const OUT_CONF0: u32 = GDMA_BASE + 0x190;
    const MEM_TRANS_EN: u32 = 1 << 4;
    const OUT_AUTO_WRBACK: u32 = 1 << 2;
    const IN_DONE: u32 = 1 << 0;
    const IN_SUC_EOF: u32 = 1 << 1;
    const OUT_DONE: u32 = 1 << 0;
    const OUT_TOTAL_EOF: u32 = 1 << 3;
    const INLINK_START: u32 = 1 << 22;
    const OUTLINK_START: u32 = 1 << 21;
    const LINK_ADDR_MASK: u32 = 0x000F_FFFF;

    // Config round-trips + M2M binding (both PERI_SELs to the same Dummy).
    wr32(IN_PERI_SEL, 1);
    wr32(OUT_PERI_SEL, 1);
    wr32(IN_CONF0, MEM_TRANS_EN);
    if rd32(IN_CONF0) & MEM_TRANS_EN == 0 {
        return Err("gdma-conf-roundtrip");
    }
    wr32(OUT_CONF0, 0x8 | OUT_AUTO_WRBACK); // OUT_EOF_MODE reset | writeback

    let (src_addr, dst_addr, tx_desc_addr, rx_desc_addr, len) = unsafe {
        let src = &raw mut DMA_SRC;
        let dst = &raw mut DMA_DST;
        let len = (*src).len() as u32;
        // owner=DMA(1) | suc_eof | length=len | size=len
        DMA_TX_DESC.dw0 = (1 << 31) | (1 << 30) | (len << 12) | len;
        DMA_TX_DESC.buffer = src as u32;
        DMA_TX_DESC.next = 0;
        DMA_RX_DESC.dw0 = (1 << 31) | len; // owner=DMA, size=len
        DMA_RX_DESC.buffer = dst as u32;
        DMA_RX_DESC.next = 0;
        (
            src as u32,
            dst as u32,
            (&raw const DMA_TX_DESC) as u32,
            (&raw const DMA_RX_DESC) as u32,
            len,
        )
    };

    wr32(IN_INT_CLR, 0xFFFF_FFFF);
    wr32(OUT_INT_CLR, 0xFFFF_FFFF);
    wr32(IN_LINK, (rx_desc_addr & LINK_ADDR_MASK) | INLINK_START);
    wr32(OUT_LINK, (tx_desc_addr & LINK_ADDR_MASK) | OUTLINK_START);

    let mut eof = false;
    for _ in 0..100_000 {
        if rd32(IN_INT_RAW) & IN_SUC_EOF != 0 {
            eof = true;
            break;
        }
    }
    if !eof {
        return Err("gdma-eof-timeout");
    }
    if rd32(IN_INT_RAW) & IN_DONE == 0 {
        return Err("gdma-in-done");
    }
    if rd32(OUT_INT_RAW) & (OUT_TOTAL_EOF | OUT_DONE) != (OUT_TOTAL_EOF | OUT_DONE) {
        return Err("gdma-out-flags");
    }

    // EOF alone is not a transfer: the bytes must actually have moved. Read
    // through raw pointers so no optimizer can fold this to the initializer.
    let moved = (0..len).all(|i| unsafe {
        core::ptr::read_volatile((src_addr + i) as *const u8)
            == core::ptr::read_volatile((dst_addr + i) as *const u8)
    });
    if !moved {
        return Err("gdma-no-m2m-model");
    }

    // Both descriptors came back to the CPU (owner=0), and the RX length
    // field holds the received byte count.
    let (tx_owner, rx_owner, rx_len) = unsafe {
        (
            (*core::ptr::addr_of!(DMA_TX_DESC)).dw0 & (1 << 31),
            (*core::ptr::addr_of!(DMA_RX_DESC)).dw0 & (1 << 31),
            ((*core::ptr::addr_of!(DMA_RX_DESC)).dw0 >> 12) & 0xFFF,
        )
    };
    if tx_owner != 0 {
        return Err("gdma-tx-owner-writeback");
    }
    if rx_owner != 0 {
        return Err("gdma-rx-owner-writeback");
    }
    if rx_len != len {
        return Err("gdma-rx-length-writeback");
    }
    Ok(())
}

// ── i2c: run a command list through the C6 I²C0 transaction engine ─────────
//
// configs/chips/esp32c6.yaml wires i2c0 @0x6000_4000 (type esp32c3_i2c) — the
// SAME command-list engine as the C3, because the C6 SVD's I2C0 register head
// (CTR@0x04, FIFO_CONF@0x18, DATA@0x1C, INT_RAW@0x20, INT_CLR@0x24,
// COMD0@0x58) is offset-identical. The C6 delta is the interrupt-matrix source
// (I2C_EXT0 = 50), which the chip yaml passes via `irq:`.
//
// What is proven: RSTART→WRITE(1)→STOP is walked to STOP; every executed slot
// latches COMD.command_done (bit 31), TRANS_COMPLETE (INT_RAW bit 7) latches,
// and CTR.TRANS_START self-clears on the launch write. With no slave wired the
// address byte is NACKed on the wire — the point is the engine ran.
//
// What is NOT claimed: with no external device, no ACK/read data path; the C3
// matrix-signal pad wiring is applied to the C6 GPIO by the shared wiring pass
// (C6 I2CEXT0 signal ids differ), so matrix-routed I²C is not proven here.
fn check_i2c() -> Result<(), &'static str> {
    const CTR: u32 = I2C0_BASE + 0x04;
    const FIFO_CONF: u32 = I2C0_BASE + 0x18;
    const DATA: u32 = I2C0_BASE + 0x1C;
    const INT_RAW: u32 = I2C0_BASE + 0x20;
    const INT_CLR: u32 = I2C0_BASE + 0x24;
    const CMD0: u32 = I2C0_BASE + 0x58;
    const TRANS_START: u32 = 1 << 5;
    const TRANS_COMPLETE: u32 = 1 << 7;
    const CMD_DONE: u32 = 1 << 31;
    // COMD word = (opcode << 11) | byte_num. opcodes: WRITE=1, STOP=2, RSTART=6.
    let cmd = |opcode: u32, byte_num: u32| (opcode << 11) | byte_num;

    wr32(INT_CLR, 0xFFFF_FFFF); // clear any stale raw-int state
    wr32(FIFO_CONF, (1 << 12) | (1 << 13)); // RX/TX FIFO reset (self-clearing)
    wr32(CMD0, cmd(6, 0)); // RSTART
    wr32(CMD0 + 4, cmd(1, 1)); // WRITE 1 byte (the address)
    wr32(CMD0 + 8, cmd(2, 0)); // STOP
    wr32(DATA, 0xA0); // address byte into TX FIFO

    if rd32(INT_RAW) & TRANS_COMPLETE != 0 {
        return Err("i2c-complete-early"); // must not be set before TRANS_START
    }
    wr32(CTR, TRANS_START);

    // TRANS_START is self-clearing: the engine consumes the strobe on the write.
    if rd32(CTR) & TRANS_START != 0 {
        return Err("i2c-start-stuck");
    }
    // Bounded poll for the transaction to finish on the wire.
    let mut completed = false;
    for _ in 0..200_000 {
        if rd32(INT_RAW) & TRANS_COMPLETE != 0 {
            completed = true;
            break;
        }
    }
    if !completed {
        return Err("i2c-no-complete");
    }
    if rd32(CMD0) & CMD_DONE == 0 {
        return Err("i2c-cmd-not-done");
    }
    Ok(())
}

// ── spi: drive a GP-SPI2 transaction through the behavioral engine ─────────
//
// configs/chips/esp32c6.yaml wires spi2 @0x6008_1000 (type esp32c3_spi) — the
// same GP-SPI transaction engine as the C3; the C6 SVD's SPI2 head (CMD@0x00,
// MS_DLEN@0x1C, DMA_INT_CLR@0x38, DMA_INT_RAW@0x3C, W0@0x98) is
// offset-identical. C6 delta: the SPI2 matrix source is 72 (C3 19), passed via
// `irq:`.
//
// What is proven: CMD.USR self-clears on completion, TRANS_DONE latches in
// DMA_INT_RAW, and W0 comes back 0xFFFF_FFFF — the idle (pulled-high) MISO
// line was shifted in over the MOSI pattern, i.e. the buffer really moved
// through the engine.
//
// What is NOT claimed: no external device on the bus (no real command/address
// phase), no DMA-coupled transfer.
fn check_spi() -> Result<(), &'static str> {
    const CMD: u32 = SPI2_BASE + 0x00;
    const MS_DLEN: u32 = SPI2_BASE + 0x1C;
    const DMA_INT_CLR: u32 = SPI2_BASE + 0x38;
    const DMA_INT_RAW: u32 = SPI2_BASE + 0x3C;
    const W0: u32 = SPI2_BASE + 0x98;
    const USR: u32 = 1 << 24;
    const TRANS_DONE: u32 = 1 << 12;

    wr32(DMA_INT_CLR, 0xFFFF_FFFF); // clear any stale raw-int state
    wr32(MS_DLEN, 32 - 1); // 32-bit (4-byte) transfer
    wr32(W0, 0x1234_5678); // MOSI payload

    if rd32(DMA_INT_RAW) & TRANS_DONE != 0 {
        return Err("spi-done-early"); // must not be set before launch
    }
    wr32(CMD, USR); // launch

    if rd32(CMD) & USR != 0 {
        return Err("spi-usr-stuck"); // USR must auto-clear on completion
    }
    if rd32(DMA_INT_RAW) & TRANS_DONE == 0 {
        return Err("spi-no-done"); // TRANS_DONE must latch
    }
    if rd32(W0) != 0xFFFF_FFFF {
        return Err("spi-no-miso"); // idle-bus MISO must overwrite W0 with 0xFF
    }
    Ok(())
}

// ── adc: one-shot SAR conversion, channel-dependent result ─────────────────
//
// configs/chips/esp32c6.yaml wires apb_saradc @0x6000_E000 (type
// esp32c3_apb_saradc); the C6 SVD's APB_SARADC one-shot surface
// (ONETIME_SAMPLE@0x20, SAR1DATA_STATUS@0x2C, INT_RAW@0x44, INT_CLR@0x4C) is
// offset-identical. C6 delta: source = 60.
//
// What is proven: ONETIME_START triggers a conversion, self-clears, latches
// SAR1 DONE in INT_RAW, and packs a channel-dependent 12-bit sample
// (0x100 + ch*0x111) plus the selected channel id into SAR1DATA_STATUS; two
// channels yield two different, predictable values.
//
// What is NOT claimed: the register reset seeds are the C3 silicon capture
// (the model's documented approximation); no external analog source, no DMA
// mode, no threshold interrupts.
fn check_adc() -> Result<(), &'static str> {
    const ONETIME_SAMPLE: u32 = APB_SARADC_BASE + 0x20;
    const SAR1DATA_STATUS: u32 = APB_SARADC_BASE + 0x2C;
    const INT_RAW: u32 = APB_SARADC_BASE + 0x44;
    const INT_CLR: u32 = APB_SARADC_BASE + 0x4C;
    const SAR1_SELECT: u32 = 1 << 31;
    const ONETIME_START: u32 = 1 << 29;
    const SAR1_DONE: u32 = 1 << 31;

    let sample = |ch: u32| -> u32 { (0x100 + ch * 0x111) & 0x0FFF };
    let oneshot = |ch: u32| SAR1_SELECT | ONETIME_START | ((ch & 0xF) << 25);

    wr32(INT_CLR, 0xFC00_0000); // clear any stale done bits

    if rd32(INT_RAW) & SAR1_DONE != 0 {
        return Err("adc-done-early"); // must not be done before a conversion
    }

    // Conversion of channel 3.
    wr32(ONETIME_SAMPLE, oneshot(3));
    if rd32(INT_RAW) & SAR1_DONE == 0 {
        return Err("adc-no-done");
    }
    if rd32(ONETIME_SAMPLE) & ONETIME_START != 0 {
        return Err("adc-start-stuck"); // START must self-clear
    }
    let d3 = rd32(SAR1DATA_STATUS);
    if d3 & 0x0FFF != sample(3) {
        return Err("adc-ch3-sample");
    }
    if (d3 >> 13) & 0xF != 3 {
        return Err("adc-ch3-channel"); // packed channel id must match
    }

    // A second conversion on a different channel must yield a different result.
    wr32(INT_CLR, 0xFC00_0000);
    wr32(ONETIME_SAMPLE, oneshot(5));
    let d5 = rd32(SAR1DATA_STATUS);
    if d5 & 0x0FFF != sample(5) {
        return Err("adc-ch5-sample");
    }
    if d3 == d5 {
        return Err("adc-channel-constant"); // result must track the channel
    }
    Ok(())
}

// ── pwm: run a LEDC timer and observe a live counter + overflow ────────────
//
// configs/chips/esp32c6.yaml wires ledc @0x6000_7000 (type esp32c6_ledc) — the
// C3 live LED PWM timer engine on the C6 field geometry. The registers it
// drives (TIMER0_CONF@0xA0, TIMER0_VALUE@0xA4, INT_RAW@0xC0, INT_CLR@0xCC) are
// offset-identical, but TIMERx_CONF is not bit-identical: DUTY_RES is [4:0],
// CLK_DIV [22:5], PAUSE 23, RST 24 (soc/esp32c6/register/soc/ledc_reg.h; the
// C3 is [3:0]/[21:4]/22/23). The gamma/event/compare/capture tail is not
// touched. C6 delta: source = 45.
//
// What is proven: with the timer released from reset the counter advances,
// wraps at the programmed 2^DUTY_RES period and latches LSTIMER0_OVF; PAUSE
// then freezes it (no further advance, no new overflow). A register file
// cannot advance, wrap and stall.
//
// What is NOT claimed: output duty on a pad (no pin electrical model here),
// the C6-only gamma/capture/event registers, or interrupt delivery.
fn check_ledc() -> Result<(), &'static str> {
    const TIMER0_CONF: u32 = LEDC_BASE + 0xA0;
    const TIMER0_VALUE: u32 = LEDC_BASE + 0xA4;
    const INT_RAW: u32 = LEDC_BASE + 0xC0;
    const INT_CLR: u32 = LEDC_BASE + 0xCC;
    const PAUSE: u32 = 1 << 23;
    const RST: u32 = 1 << 24;
    const LSTIMER0_OVF: u32 = 1 << 0;
    // CONF = DUTY_RES | (CLK_DIV_field << 5); CLK_DIV integer part = field>>8.
    let conf = |duty_res: u32, div_int: u32| (duty_res & 0x1F) | (((div_int & 0x3FF) << 8) << 5);

    // DUTY_RES = 14 → period 16384 counts at divider 1 (same choice as the C3
    // fixture: many distinct sample points per period, so the counter is
    // observable across the bus tick quantum).
    wr32(TIMER0_CONF, conf(14, 1) | RST);
    wr32(INT_CLR, LSTIMER0_OVF);
    wr32(TIMER0_CONF, conf(14, 1)); // release reset, start counting

    if rd32(INT_RAW) & LSTIMER0_OVF != 0 {
        return Err("ledc-ovf-early"); // must not have wrapped yet
    }

    // (1) The live counter advances with elapsed cycles.
    let a = rd32(TIMER0_VALUE);
    let mut advanced = false;
    for _ in 0..200_000 {
        if rd32(TIMER0_VALUE) != a {
            advanced = true;
            break;
        }
    }
    if !advanced {
        return Err("ledc-not-counting");
    }

    // (2) Run long enough to wrap the period and latch overflow.
    let mut overflowed = false;
    for _ in 0..200_000 {
        if rd32(INT_RAW) & LSTIMER0_OVF != 0 {
            overflowed = true;
            break;
        }
    }
    if !overflowed {
        return Err("ledc-ovf-timeout");
    }

    // (3) PAUSE freezes the counter: after clearing overflow no new wrap fires.
    wr32(TIMER0_CONF, conf(14, 1) | PAUSE);
    wr32(INT_CLR, LSTIMER0_OVF);
    let p1 = rd32(TIMER0_VALUE);
    for i in 0u32..4_000 {
        core::hint::black_box(i);
    }
    let p2 = rd32(TIMER0_VALUE);
    if p1 != p2 {
        return Err("ledc-pause-not-frozen");
    }
    if rd32(INT_RAW) & LSTIMER0_OVF != 0 {
        return Err("ledc-pause-still-overflowing");
    }
    Ok(())
}

// ── wdt: the TIMG0 MWDT write lock, config surface and stage chain ─────────
//
// configs/chips/esp32c6.yaml wires timg0 @0x6000_8000 as `esp32c6_mwdt` (the
// shared esp32 TIMG with `with_mwdt`). C3/C6 MWDT register layout:
// WDTCONFIG0@0x48 (EN bit31; STG0 [30:29], STG1 [28:27]), WDTCONFIG1@0x4C
// (prescaler), WDTCONFIG2@0x50 (STG0_HOLD), WDTCONFIG3@0x54 (STG1_HOLD),
// WDTCONFIG4/5 (STG2/3_HOLD), WDTFEED@0x60, WDTWPROTECT@0x64,
// INT_RAW_TIMERS@0x74 (WDT bit1), INT_CLR_TIMERS@0x7C.
//
// What is proven:
//   * WDTWPROTECT resets to the unlock key (0x50D8_3AA1); writing a different
//     value LOCKS the WDTCONFIG0..5 surface — a locked config write is dropped
//     (readback unchanged), while WDTFEED/WDTWPROTECT stay writable.
//   * WDTCONFIG1 round-trips while unlocked, and WDTCONFIG0 stores EN plus the
//     stage actions.
//   * With WDTCONFIG2 (stage-0 hold) programmed, the countdown expires and
//     latches INT_RAW_TIMERS.WDT_INT_RAW; INT_CLR_TIMERS is W1C.
//   * A WDTFEED write restarts the chain at stage 0, and a second expiry
//     latches again.
//   * The stage chain progresses: with STG1 also configured as an interrupt
//     stage, a second latch arrives WITHOUT a feed — only the chain advancing
//     into stage 1 and counting STG1_HOLD can produce it. Another feed then
//     restarts stage 0 (the model is at stage 2 by then, whose seeded SVD hold
//     is ~1M walk ticks, so a latch inside the stage-0 budget proves the feed
//     reset the stage pointer, not that a later stage happened to expire).
//
// What is NOT claimed: the CPU/system reset actions (STG = 2/3 advance the
// chain but the model never resets — no safe bus reset-request path exists),
// the silicon 12.5 ns × prescaler timeout rate (the mwdt variant is
// walk-driven, so the hold counts peripheral walk ticks — 512 CPU cycles each
// under the CLI's default tick interval), or interrupt-matrix delivery of the
// WDT latch — INT_RAW_TIMERS/INT_ST_TIMERS only. The hold values below are
// chosen to be observable inside the fixture's step budget, not to be a wall
// clock timeout.
fn check_wdt() -> Result<(), &'static str> {
    const WDT_CONFIG0: u32 = TIMG0_BASE + 0x48;
    const WDT_CONFIG1: u32 = TIMG0_BASE + 0x4C;
    const WDT_CONFIG2: u32 = TIMG0_BASE + 0x50;
    const WDT_CONFIG3: u32 = TIMG0_BASE + 0x54;
    const WDT_FEED: u32 = TIMG0_BASE + 0x60;
    const WDT_WPROTECT: u32 = TIMG0_BASE + 0x64;
    const INT_RAW_TIMERS: u32 = TIMG0_BASE + 0x74;
    const INT_CLR_TIMERS: u32 = TIMG0_BASE + 0x7C;
    const WDT_WKEY: u32 = 0x50D8_3AA1;
    const WDT_EN: u32 = 1 << 31;
    const WDT_STG0_INT: u32 = 1 << 29; // STG0 field [30:29] = 1 (interrupt)
    const WDT_STG1_INT: u32 = 1 << 27; // STG1 field [28:27] = 1 (interrupt)
    const WDT_INT_RAW: u32 = 1 << 1;
    const STG0_HOLD: u32 = 1000; // walk ticks (512 cycles each) ≈ 0.51M cycles
    const STG1_HOLD: u32 = 800; // stage-1 hold; chain latch at STG0+STG1

    // (1) Reset state: unlocked (WDTWPROTECT holds the key).
    if rd32(WDT_WPROTECT) != WDT_WKEY {
        return Err("wdt-wprotect-reset");
    }

    // (2) Lock it and prove config writes are dropped.
    wr32(WDT_WPROTECT, 0);
    wr32(WDT_CONFIG0, WDT_EN | WDT_STG0_INT);
    if rd32(WDT_CONFIG0) & WDT_EN != 0 {
        return Err("wdt-locked-config-write-not-dropped");
    }
    wr32(WDT_CONFIG2, 0x1234_5678);
    if rd32(WDT_CONFIG2) == 0x1234_5678 {
        return Err("wdt-locked-hold-write-not-dropped");
    }

    // (3) Unlock and prove the config surface round-trips.
    wr32(WDT_WPROTECT, WDT_WKEY);
    wr32(WDT_CONFIG1, 0x0001_2345);
    if rd32(WDT_CONFIG1) != 0x0001_2345 {
        return Err("wdt-config1-roundtrip");
    }

    // (4) Arm stage 0 from a known hold count. Writing CONFIG0 with EN clear
    //     first makes the re-enable a true 0→1 edge on the model.
    wr32(WDT_CONFIG2, STG0_HOLD);
    wr32(INT_CLR_TIMERS, WDT_INT_RAW);
    wr32(WDT_CONFIG0, WDT_STG0_INT); // EN clear: not counting
    wr32(WDT_CONFIG0, WDT_EN | WDT_STG0_INT); // EN set: arm from hold
    if rd32(WDT_CONFIG0) & WDT_EN == 0 {
        return Err("wdt-config0-roundtrip");
    }
    if rd32(INT_RAW_TIMERS) & WDT_INT_RAW != 0 {
        return Err("wdt-raw-early");
    }

    // (5) Expiry latch. Pure reads on purpose: the mwdt variant is walk-driven
    //     exactly so a read-only status poll observes time passing.
    let mut latched = false;
    for _ in 0..500_000 {
        if rd32(INT_RAW_TIMERS) & WDT_INT_RAW != 0 {
            latched = true;
            break;
        }
    }
    if !latched {
        return Err("wdt-no-expiry");
    }

    // (6) INT_CLR must clear the latch (W1C).
    wr32(INT_CLR_TIMERS, WDT_INT_RAW);
    if rd32(INT_RAW_TIMERS) & WDT_INT_RAW != 0 {
        return Err("wdt-intclr-not-w1c");
    }

    // (7) Stage chain. Configure stage 1 as a second interrupt stage and feed
    //     to restart at stage 0. After the stage-0 latch the next latch within
    //     this budget can only be stage 1: the stages after it carry their
    //     long seeded SVD holds, so nothing loops back to stage 0 in time.
    wr32(WDT_CONFIG3, STG1_HOLD);
    wr32(WDT_CONFIG0, WDT_EN | WDT_STG0_INT | WDT_STG1_INT);
    wr32(WDT_FEED, WDT_WKEY);

    let mut stage0 = false;
    for _ in 0..500_000 {
        if rd32(INT_RAW_TIMERS) & WDT_INT_RAW != 0 {
            stage0 = true;
            break;
        }
    }
    if !stage0 {
        return Err("wdt-stage0-no-expiry");
    }
    wr32(INT_CLR_TIMERS, WDT_INT_RAW);
    if rd32(INT_RAW_TIMERS) & WDT_INT_RAW != 0 {
        return Err("wdt-stage0-intclr");
    }

    let mut stage1 = false;
    for _ in 0..500_000 {
        if rd32(INT_RAW_TIMERS) & WDT_INT_RAW != 0 {
            stage1 = true;
            break;
        }
    }
    if !stage1 {
        return Err("wdt-stage1-no-progress");
    }
    wr32(INT_CLR_TIMERS, WDT_INT_RAW);

    // (8) After the stage-1 latch the model sits in stage 2 (its seeded SVD
    //     hold is ~1M walk ticks). A feed must restart stage 0, so a latch
    //     arrives inside the stage-0 budget again.
    wr32(WDT_FEED, WDT_WKEY);
    let mut refed = false;
    for _ in 0..500_000 {
        if rd32(INT_RAW_TIMERS) & WDT_INT_RAW != 0 {
            refed = true;
            break;
        }
    }
    if !refed {
        return Err("wdt-feed-not-restaging");
    }

    // Leave the block quiet for the rest of the run.
    wr32(WDT_CONFIG0, WDT_STG0_INT | WDT_STG1_INT); // EN clear: disable
    wr32(INT_CLR_TIMERS, WDT_INT_RAW);
    Ok(())
}

// ── rtc: the LP_TIMER main counter snapshot protocol ───────────────────────
//
// configs/chips/esp32c6.yaml wires lp_timer @0x600B_0C00 (type
// `esp32c6_lp_rtc`, model `esp32c6::lp_timer::Esp32c6LpTimer`). This is NOT
// the C3's RTC_CNTL timer: the C6 keeps RTC time in LP_TIMER, whose snapshot
// surface is UPDATE@0x10 (MAIN_TIMER_UPDATE, bit 28), MAIN_BUF0 @0x14/0x18 and
// MAIN_BUF1 @0x1C/0x20.
//
// What is proven (the IDF `lp_timer_hal_get_cycle_count` protocol): the
// UPDATE strobe latches the live 48-bit counter into MAIN_BUF0, the readout
// stays frozen until the next strobe, a second strobe after elapsed cycles
// yields a strictly greater value, and it shifts the previous MAIN_BUF0 into
// MAIN_BUF1 (the LL's documented double buffer).
//
// What is NOT claimed: the absolute RTC-slow rate (the counter advances one
// step per elapsed CPU cycle; no 32.768 kHz / RC_SLOW modelling), TAR0/TAR1
// alarm comparators, overflow/wakeup interrupts, or retention across sleep.
fn check_rtc() -> Result<(), &'static str> {
    const UPDATE: u32 = LP_TIMER_BASE + 0x10;
    const BUF0_LOW: u32 = LP_TIMER_BASE + 0x14;
    const BUF0_HIGH: u32 = LP_TIMER_BASE + 0x18;
    const BUF1_LOW: u32 = LP_TIMER_BASE + 0x1C;
    const BUF1_HIGH: u32 = LP_TIMER_BASE + 0x20;
    const MAIN_TIMER_UPDATE: u32 = 1 << 28;

    let snapshot = || -> u64 {
        wr32(UPDATE, MAIN_TIMER_UPDATE);
        ((rd32(BUF0_HIGH) as u64) << 32) | rd32(BUF0_LOW) as u64
    };
    let buf1 = || -> u64 { ((rd32(BUF1_HIGH) as u64) << 32) | rd32(BUF1_LOW) as u64 };

    let v1 = snapshot();
    // Readout is latched: re-reading without a strobe must not move.
    if snapshot_read_frozen(BUF0_LOW, BUF0_HIGH, v1) {
        return Err("rtc-readout-not-latched");
    }

    for i in 0u32..20_000 {
        core::hint::black_box(i);
    }
    let v2 = snapshot();
    if v2 <= v1 {
        return Err("rtc-not-counting");
    }

    // The second strobe shifted the first snapshot into MAIN_BUF1.
    if buf1() != v1 {
        return Err("rtc-buf1-shift");
    }
    Ok(())
}

/// Re-read MAIN_BUF0 without strobing UPDATE and report whether it changed.
fn snapshot_read_frozen(low: u32, high: u32, expected: u64) -> bool {
    let again = ((rd32(high) as u64) << 32) | rd32(low) as u64;
    again != expected
}

#[entry]
fn main() -> ! {
    // Order: clock first (exercises the PCR gate, reopening it before any
    // console output), then the six bring-up classes, then the per-peripheral
    // estate. Every class is attempted against a declared peripheral; nothing
    // is faked.
    report("clock", check_clock());
    report("gpio", check_gpio());
    report("timer", check_timer());
    report("dma", check_dma());
    report("irq", check_irq());
    report("i2c", check_i2c());
    report("spi", check_spi());
    report("adc", check_adc());
    report("pwm", check_ledc());
    report("wdt", check_wdt());
    report("rtc", check_rtc());
    uart0_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
