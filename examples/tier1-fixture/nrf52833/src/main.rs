//! nRF52833 Tier-1 fixture firmware.
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses and reports one line per class over UARTE0 using the
//! TIER1 protocol:
//!
//! ```text
//! TIER1 <class> PASS
//! TIER1 <class> FAIL code=<reason>
//! TIER1 done
//! ```
//!
//! The `uart` class is implicit: receiving `TIER1 done` over UARTE0 is itself
//! the proof of a working UART path.
//!
//! The nRF52833 chip YAML declares `uart0`, `uart1`, `gpio0` + `gpio1`,
//! `clock`, `timer0`–`timer4`, `rtc0`–`rtc2`, `pwm0`–`pwm3`, `twi1`, `spi2`,
//! `spi3`, `saadc`, `wdt`, and the Cortex-M4F `nvic`. Two classes are proven
//! here beyond the family precedent (nrf52832/nrf52840 render both `na`):
//!
//! * `irq` — a real peripheral-sourced interrupt: TIMER0 COMPARE0 (INTENSET
//!   bit 16) pends NVIC IRQ 8, and the `DefaultHandler` below counts the
//!   vector actually running.
//! * `dma` — nRF52 DMA is **EasyDMA** (an engine integrated into
//!   UARTE/SPIM/TWIM/PWM/SAADC, not a central controller). The chip YAML opts
//!   the class in explicitly (`tier1_classes: ["dma"]`); the check proves
//!   EasyDMA descriptor semantics on the SAADC RESULT channel: pointer,
//!   length, and the transferred payload, with sentinels proving MAXCNT is
//!   honoured.
//!
//! The peripheral map is an nRF52840 subset at the same bases with the same
//! silicon IP, so the register sequences mirror the proven nrf52840 fixture.
//! Register offsets follow the nRF52833 Product Specification v1.7 (shared
//! UARTE/GPIO/TIMER/RTC/TWIM/SPIM/SAADC/WDT/PWM layouts; identical to the
//! nRF52840 PS v1.7 §6 maps).
//!
//! Chosen peripheral instances, cross-checked against
//! `configs/chips/nrf52833.yaml`:
//!   CLOCK 0x40000000, GPIO P0 0x50000000, GPIO P1 0x50001000,
//!   TIMER0 0x40008000, RTC0 0x4000B000, TWIM1 0x40004000,
//!   SPIM2 0x40023000, SAADC 0x40007000, WDT 0x40010000, PWM0 0x4001C000.

#![no_std]
#![no_main]

use cortex_m_rt::{entry, exception};
use panic_halt as _;
use tier1_fixture_common::{rd32 as reg_read, wr32 as reg_write};

// ── UARTE0 (nRF52833 PS v1.7 §6.33, base 0x40002000) ──────────────────────
//
// The chip YAML models uart0 as the nRF52840 UARTE (EasyDMA), not the legacy
// non-DMA UART: there is no byte-at-a-time TXD register, transmission is
// PSEL + BAUDRATE + ENABLE=8 then TXD.PTR / TXD.MAXCNT / TASKS_STARTTX, with
// completion at EVENTS_ENDTX (0x120).
const UART0_BASE: u32 = 0x4000_2000;
const UART0_TASK_STARTTX: u32 = UART0_BASE + 0x008;
const UART0_EVENTS_ENDTX: u32 = UART0_BASE + 0x120;
const UART0_ENABLE: u32 = UART0_BASE + 0x500;
const UART0_PSEL_TXD: u32 = UART0_BASE + 0x50C;
const UART0_PSEL_RXD: u32 = UART0_BASE + 0x514;
const UART0_BAUDRATE: u32 = UART0_BASE + 0x524;
const UART0_TXD_PTR: u32 = UART0_BASE + 0x544;
const UART0_TXD_MAXCNT: u32 = UART0_BASE + 0x548;

const UARTE_ENABLE: u32 = 8; // PS: 8 = UARTE (EasyDMA); 4 = legacy UART
const BAUDRATE_115200: u32 = 0x01D6_0000; // round(115200 * 2^32 / 16 MHz)
// micro:bit v2 console wiring: target TX = P0.06, target RX = P1.08.
// PSEL fields: PIN [4:0], PORT [5], CONNECT bit 31 (0 = connected).
const PSEL_TXD_P0_06: u32 = 6;
const PSEL_RXD_P1_08: u32 = (1 << 5) | 8;

// ── GPIO P0 (base 0x50000000) ─────────────────────────────────────────────
//
// OUT     offset 0x504 — output register (read current output state).
// OUTSET  offset 0x508 — write 1 to set pins high.
// OUTCLR  offset 0x50C — write 1 to clear pins low.
// DIRSET  offset 0x518 — write 1 to configure pins as output.
const GPIO0_BASE: u32 = 0x5000_0000;
const GPIO0_OUT: u32 = GPIO0_BASE + 0x504;
const GPIO0_OUTSET: u32 = GPIO0_BASE + 0x508;
const GPIO0_OUTCLR: u32 = GPIO0_BASE + 0x50C;
const GPIO0_DIRSET: u32 = GPIO0_BASE + 0x518;

// ── GPIO P1 (silicon block base 0x50000300; P1.00–P1.09) ─────────────────
// The descriptor registers a compact window starting at block offset 0x500;
// guest code uses the unmodified silicon addresses.
const GPIO1_BASE: u32 = 0x5000_0300;
const GPIO1_OUT: u32 = GPIO1_BASE + 0x504;
const GPIO1_OUTSET: u32 = GPIO1_BASE + 0x508;
const GPIO1_OUTCLR: u32 = GPIO1_BASE + 0x50C;
const GPIO1_DIRSET: u32 = GPIO1_BASE + 0x518;

// ── NVIC (declared in configs/chips/nrf52833.yaml, base 0xE000E100) ───────
//
// The NVIC is part of the Cortex-M4F System Control Space; the engine installs
// it for every Cortex-M chip, and the chip YAML now declares it so the tier-1
// `irq` class resolves to this chip's row. TIMER0's IRQ line is 8 (nrfx
// nrf52833.svd `TIMER0_IRQn`, matches the YAML `irq: 8` on timer0).
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ICPR0: u32 = 0xE000_E280;
const TIMER0_IRQ: u32 = 8;

// ── CLOCK (nrf_clock, base 0x40000000) ────────────────────────────────────
const CLOCK_BASE: u32 = 0x4000_0000;
const CLOCK_TASKS_HFCLKSTART: u32 = CLOCK_BASE;
const CLOCK_EVENTS_HFCLKSTARTED: u32 = CLOCK_BASE + 0x100;
const CLOCK_HFCLKRUN: u32 = CLOCK_BASE + 0x408;

// ── TIMER0 (nrf52840_timer, base 0x40008000) ──────────────────────────────
const TIMER0_BASE: u32 = 0x4000_8000;
const TIMER0_TASKS_START: u32 = TIMER0_BASE;
const TIMER0_TASKS_STOP: u32 = TIMER0_BASE + 0x004;
const TIMER0_TASKS_CLEAR: u32 = TIMER0_BASE + 0x00C;
const TIMER0_TASKS_CAPTURE0: u32 = TIMER0_BASE + 0x040;
const TIMER0_EVENTS_COMPARE0: u32 = TIMER0_BASE + 0x140;
const TIMER0_INTENSET: u32 = TIMER0_BASE + 0x304;
const TIMER0_INTENCLR: u32 = TIMER0_BASE + 0x308;
const TIMER0_MODE: u32 = TIMER0_BASE + 0x504;
const TIMER0_BITMODE: u32 = TIMER0_BASE + 0x508;
const TIMER0_PRESCALER: u32 = TIMER0_BASE + 0x510;
const TIMER0_CC0: u32 = TIMER0_BASE + 0x540;

// ── RTC0 (nrf52840_rtc, base 0x4000B000) ──────────────────────────────────
const RTC0_BASE: u32 = 0x4000_B000;
const RTC0_TASKS_START: u32 = RTC0_BASE;
const RTC0_TASKS_CLEAR: u32 = RTC0_BASE + 0x008;
const RTC0_COUNTER: u32 = RTC0_BASE + 0x504;
const RTC0_PRESCALER: u32 = RTC0_BASE + 0x508;

// ── TWI1 / I2C (nrf52840_i2c → TWIM, base 0x40004000) ─────────────────────
// EasyDMA master. With no device attached at ADDRESS, the modeled engine
// runs the transfer, reports an address-NACK (ERRORSRC.ANACK), and still
// fires EVENTS_LASTTX — a genuine modeled round-trip, not a stub.
const TWI1_BASE: u32 = 0x4000_4000;
const TWI1_TASKS_STARTTX: u32 = TWI1_BASE + 0x008;
const TWI1_EVENTS_ERROR: u32 = TWI1_BASE + 0x124;
const TWI1_EVENTS_LASTTX: u32 = TWI1_BASE + 0x160;
const TWI1_ERRORSRC: u32 = TWI1_BASE + 0x4C4;
const TWI1_ENABLE: u32 = TWI1_BASE + 0x500;
const TWI1_TXD_PTR: u32 = TWI1_BASE + 0x544;
const TWI1_TXD_MAXCNT: u32 = TWI1_BASE + 0x548;
const TWI1_TXD_AMOUNT: u32 = TWI1_BASE + 0x54C;
const TWI1_ADDRESS: u32 = TWI1_BASE + 0x588;
const ERRORSRC_ANACK: u32 = 1 << 1;

// ── SPI2 (nrf52840_spi → SPIM EasyDMA, base 0x40023000) ───────────────────
// The chip YAML pins `profile: nrf52_spim`, so these offsets are the Nordic
// SPIM map (not STM32's). SPIM3 (0x4002F000) is the same IP at a second
// window but is not attempted here.
const SPI2_BASE: u32 = 0x4002_3000;
const SPI2_TASKS_START: u32 = SPI2_BASE + 0x010;
const SPI2_EVENTS_END: u32 = SPI2_BASE + 0x118;
const SPI2_ENABLE: u32 = SPI2_BASE + 0x500;
const SPI2_RXD_PTR: u32 = SPI2_BASE + 0x534;
const SPI2_RXD_MAXCNT: u32 = SPI2_BASE + 0x538;
const SPI2_RXD_AMOUNT: u32 = SPI2_BASE + 0x53C;
const SPI2_TXD_PTR: u32 = SPI2_BASE + 0x544;
const SPI2_TXD_MAXCNT: u32 = SPI2_BASE + 0x548;
const SPI2_TXD_AMOUNT: u32 = SPI2_BASE + 0x54C;

// ── SAADC (nrf52840_saadc, base 0x40007000) ───────────────────────────────
// 12-bit ADC with EasyDMA RESULT buffer. The modeled engine performs a
// deterministic conversion: TASKS_START → STARTED, TASKS_SAMPLE writes
// one enabled-channel scan per SAMPLE; END fires only at buffer-full.
const SAADC_BASE: u32 = 0x4000_7000;
const SAADC_TASKS_START: u32 = SAADC_BASE;
const SAADC_TASKS_SAMPLE: u32 = SAADC_BASE + 0x004;
const SAADC_EVENTS_STARTED: u32 = SAADC_BASE + 0x100;
const SAADC_EVENTS_END: u32 = SAADC_BASE + 0x104;
const SAADC_EVENTS_RESULTDONE: u32 = SAADC_BASE + 0x10C;
const SAADC_ENABLE: u32 = SAADC_BASE + 0x500;
const SAADC_CH0_PSELP: u32 = SAADC_BASE + 0x510;
const SAADC_CH0_CONFIG: u32 = SAADC_BASE + 0x518;
const SAADC_RESOLUTION: u32 = SAADC_BASE + 0x5F0;
const SAADC_RESULT_PTR: u32 = SAADC_BASE + 0x62C;
const SAADC_RESULT_MAXCNT: u32 = SAADC_BASE + 0x630;
const SAADC_RESULT_AMOUNT: u32 = SAADC_BASE + 0x634;
// Explicit modeled VDD source: floor((3.3V/3.6V) * 2^N), default gain/ref.
const SAADC_CODE_12BIT: u16 = 3754; // modeled VDD(3.3V)/3.6V * 2^12
const SAADC_CODE_10BIT: u16 = 938; // modeled VDD(3.3V)/3.6V * 2^10

// ── WDT (nrf52840_watchdog, base 0x40010000) ──────────────────────────────
const WDT_BASE: u32 = 0x4001_0000;
const WDT_TASKS_START: u32 = WDT_BASE;
const WDT_EVENTS_TIMEOUT: u32 = WDT_BASE + 0x100;
const WDT_RUNSTATUS: u32 = WDT_BASE + 0x400;
const WDT_CRV: u32 = WDT_BASE + 0x504;
const WDT_RREN: u32 = WDT_BASE + 0x508;

// PWM0 (nRF52833 PS v1.7 §6.17, base 0x4001C000; same model as nRF52840). The
// sequence engine reads SEQ[0].CNT 16-bit duty values out of guest RAM at
// SEQ[0].PTR (EasyDMA-style) and fires SEQSTARTED0 / SEQEND0 / PWMPERIODEND.
const PWM0_BASE: u32 = 0x4001_C000;
const PWM0_TASKS_SEQSTART0: u32 = PWM0_BASE + 0x008;
const PWM0_EVENTS_SEQSTARTED0: u32 = PWM0_BASE + 0x108;
const PWM0_EVENTS_SEQEND0: u32 = PWM0_BASE + 0x110;
const PWM0_EVENTS_PWMPERIODEND: u32 = PWM0_BASE + 0x118;
const PWM0_ENABLE: u32 = PWM0_BASE + 0x500;
const PWM0_MODE: u32 = PWM0_BASE + 0x504;
const PWM0_COUNTERTOP: u32 = PWM0_BASE + 0x508;
const PWM0_PRESCALER: u32 = PWM0_BASE + 0x50C;
const PWM0_DECODER: u32 = PWM0_BASE + 0x510;
const PWM0_LOOP: u32 = PWM0_BASE + 0x514;
const PWM0_SEQ0_PTR: u32 = PWM0_BASE + 0x520;
const PWM0_SEQ0_CNT: u32 = PWM0_BASE + 0x524;
const PWM0_SEQ0_REFRESH: u32 = PWM0_BASE + 0x528;
const PWM0_SEQ0_ENDDELAY: u32 = PWM0_BASE + 0x52C;
const PWM0_PSEL_OUT0: u32 = PWM0_BASE + 0x560;

// ── UARTE0 TX buffer (EasyDMA reads from RAM, never flash) ────────────────
// Static .bss RAM, addressable by the EasyDMA engine. One shared 64-byte
// buffer is safe: each transfer is drained before the next starts (ENDTX
// poll), matching the nrf52840 fixture.
static mut TX_BUF: [u8; 64] = [0; 64];

// EasyDMA buffers for the TWIM (I2C) and SPIM (SPI) checks. Static .bss RAM,
// the only region the EasyDMA engines can address.
static mut I2C_TX_BUF: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
static mut SPI_TX_BUF: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
static mut SPI_RX_BUF: [u8; 4] = [0; 4];

// SAADC RESULT buffer (4 x 16-bit samples). Static .bss RAM.
static mut ADC_RESULT_BUF: [u16; 4] = [0; 4];

// PWM SEQ[0] duty buffer (4 x 16-bit). Static .bss RAM — the sequence engine
// reads these duty values out by EasyDMA at SEQ[0].PTR.
static mut PWM_SEQ_BUF: [u16; 4] = [0x8000 | 250, 0x8000 | 500, 0x8000 | 750, 0x8000 | 1000];

// Two destination buffers for the EasyDMA descriptor proof. Both are
// sentinel-prefilled by the check before each transfer, so the check can tell
// exactly which words the engine wrote (payload) and which it left alone
// (MAXCNT honoured).
static mut DMA_BUF_A: [u16; 4] = [0xBEEF; 4];
static mut DMA_BUF_B: [u16; 4] = [0xBEEF; 4];
const DMA_SENTINEL: u16 = 0xBEEF;

// Incremented by the TIMER0 vector handler; the irq check polls it.
static mut TIMER0_IRQ_HITS: u32 = 0;

/// Spin until the event register at `addr` reads non-zero, or give up.
/// Returns true if the event fired. Each loop iteration steps the CPU, which
/// ticks the peripherals, so the modeled HW makes progress while we wait.
fn poll_event(addr: u32) -> bool {
    let mut spins = 0u32;
    while reg_read(addr) == 0 {
        spins += 1;
        if spins > 1_000_000 {
            return false;
        }
    }
    true
}

// ── UARTE0 output (EasyDMA buffer per line, ENDTX-polled) ─────────────────
fn uart_dma(bytes: &[u8]) {
    let n = core::cmp::min(bytes.len(), 64);
    unsafe {
        let buf = core::ptr::addr_of_mut!(TX_BUF) as *mut u8;
        for (i, byte) in bytes.iter().take(n).enumerate() {
            core::ptr::write_volatile(buf.add(i), *byte);
        }
        // Clear any stale completion event, then arm + start the EasyDMA TX.
        reg_write(UART0_EVENTS_ENDTX, 0);
        reg_write(UART0_TXD_PTR, buf as u32);
        reg_write(UART0_TXD_MAXCNT, n as u32);
        reg_write(UART0_TASK_STARTTX, 1);
        // The transfer completes on the next bus tick; wait for ENDTX so we
        // never overwrite the buffer before the engine has DMAed it out.
        let mut spins = 0u32;
        while reg_read(UART0_EVENTS_ENDTX) == 0 {
            spins += 1;
            if spins > 1_000_000 {
                break;
            }
        }
        reg_write(UART0_EVENTS_ENDTX, 0);
    }
}

fn uart_write_str(s: &str) {
    uart_dma(s.as_bytes());
}

fn uart_write_line(s: &str) {
    uart_write_str(s);
    uart_write_str("\r\n");
}

fn report(class: &str, result: Result<(), &'static str>) {
    uart_write_str("TIER1 ");
    uart_write_str(class);
    match result {
        Ok(()) => uart_write_line(" PASS"),
        Err(code) => {
            uart_write_str(" FAIL code=");
            uart_write_line(code);
        }
    }
}

// ── gpio: DIRSET + OUTSET/OUTCLR read-back on BOTH ports ──────────────────
//
// P0.13 (no boot strap on the family) and P1.05 (inside the 10-pin P1 range
// the descriptor declares). Testing P1 exercises the unmodified
// silicon P1 block at 0x50000300, with OUT at block offset 0x504.
fn check_gpio() -> Result<(), &'static str> {
    const PIN0: u32 = 1 << 13;
    const PIN1: u32 = 1 << 5;

    reg_write(GPIO0_DIRSET, PIN0);
    reg_write(GPIO0_OUTSET, PIN0);
    if reg_read(GPIO0_OUT) & PIN0 == 0 {
        return Err("gpio-p0-outset");
    }
    reg_write(GPIO0_OUTCLR, PIN0);
    if reg_read(GPIO0_OUT) & PIN0 != 0 {
        return Err("gpio-p0-outclr");
    }

    reg_write(GPIO1_DIRSET, PIN1);
    reg_write(GPIO1_OUTSET, PIN1);
    if reg_read(GPIO1_OUT) & PIN1 == 0 {
        return Err("gpio-p1-outset");
    }
    reg_write(GPIO1_OUTCLR, PIN1);
    if reg_read(GPIO1_OUT) & PIN1 != 0 {
        return Err("gpio-p1-outclr");
    }
    if reg_read(GPIO0_OUT) & PIN1 != 0 {
        return Err("gpio-p1-bleed");
    }
    Ok(())
}

// ── clock: TASKS_HFCLKSTART → EVENTS_HFCLKSTARTED + HFCLKRUN ───────────────
fn check_clock() -> Result<(), &'static str> {
    reg_write(CLOCK_EVENTS_HFCLKSTARTED, 0);
    reg_write(CLOCK_TASKS_HFCLKSTART, 1);
    if !poll_event(CLOCK_EVENTS_HFCLKSTARTED) {
        return Err("clock-no-hfclkstarted");
    }
    if reg_read(CLOCK_HFCLKRUN) & 1 == 0 {
        return Err("clock-hfclkrun");
    }
    Ok(())
}

// ── timer: free-running counter advances, sampled via TASKS_CAPTURE ────────
fn check_timer() -> Result<(), &'static str> {
    reg_write(TIMER0_MODE, 0); // Timer mode
    reg_write(TIMER0_BITMODE, 3); // 32-bit
    reg_write(TIMER0_PRESCALER, 0); // 1:1
    reg_write(TIMER0_TASKS_CLEAR, 1);
    reg_write(TIMER0_TASKS_START, 1);

    // Let it run, then capture.
    for _ in 0..256 {
        core::hint::spin_loop();
    }
    reg_write(TIMER0_TASKS_CAPTURE0, 1);
    let c1 = reg_read(TIMER0_CC0);
    if c1 == 0 {
        return Err("timer-not-advancing");
    }

    // Capture again later: counter must have moved forward.
    for _ in 0..256 {
        core::hint::spin_loop();
    }
    reg_write(TIMER0_TASKS_CAPTURE0, 1);
    let c2 = reg_read(TIMER0_CC0);
    if c2 <= c1 {
        return Err("timer-no-progress");
    }
    Ok(())
}

// ── irq: peripheral-sourced NVIC delivery ─────────────────────────────────
//
// TIMER0 is armed so its COMPARE0 match raises EVENTS_COMPARE0 and, with
// INTENSET bit 16 set, pends the peripheral's NVIC line (IRQ 8 — the YAML
// `irq: 8` on timer0). The firmware enables the line in NVIC_ISER0 and then
// waits for the `DefaultHandler` below to count the vector. This is a REAL
// interrupt taken through the core's exception path, not a status-flag poll:
// the handler runs only if the engine pends the line and the CPU vectors to
// it. The handler acks the peripheral (INTENCLR + EVENTS_COMPARE0=0) and the
// NVIC (ICER/ICPR) before returning so it cannot re-enter.
fn check_irq() -> Result<(), &'static str> {
    // Park the timer (the timer check above left it running) and clear state.
    reg_write(TIMER0_TASKS_STOP, 1);
    reg_write(TIMER0_TASKS_CLEAR, 1);
    reg_write(TIMER0_MODE, 0);
    reg_write(TIMER0_BITMODE, 3);
    reg_write(TIMER0_PRESCALER, 0);
    reg_write(TIMER0_EVENTS_COMPARE0, 0);
    reg_write(TIMER0_INTENCLR, 1 << 16);
    reg_write(NVIC_ICER0, 1 << TIMER0_IRQ);
    reg_write(NVIC_ICPR0, 1 << TIMER0_IRQ);

    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(TIMER0_IRQ_HITS), 0) };

    reg_write(TIMER0_CC0, 32);
    reg_write(NVIC_ISER0, 1 << TIMER0_IRQ);
    reg_write(TIMER0_INTENSET, 1 << 16);
    reg_write(TIMER0_TASKS_START, 1);

    let mut delivered = false;
    for _ in 0..1_000_000u32 {
        if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TIMER0_IRQ_HITS)) } != 0 {
            delivered = true;
            break;
        }
        core::hint::spin_loop();
    }

    reg_write(TIMER0_TASKS_STOP, 1);
    reg_write(TIMER0_INTENCLR, 1 << 16);
    reg_write(NVIC_ICER0, 1 << TIMER0_IRQ);
    reg_write(NVIC_ICPR0, 1 << TIMER0_IRQ);
    if !delivered {
        return Err("irq-not-delivered");
    }
    Ok(())
}

/// Vector handler. `irqn` is the external interrupt number, so this counts
/// only the TIMER0 line; any other exception falls through (unexpected here).
#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn as u32 == TIMER0_IRQ {
        reg_write(TIMER0_INTENCLR, 1 << 16);
        reg_write(TIMER0_EVENTS_COMPARE0, 0);
        reg_write(NVIC_ICER0, 1 << TIMER0_IRQ);
        reg_write(NVIC_ICPR0, 1 << TIMER0_IRQ);
        let hits = core::ptr::read_volatile(core::ptr::addr_of!(TIMER0_IRQ_HITS));
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TIMER0_IRQ_HITS), hits + 1);
    }
}

// ── rtc: TASKS_START → COUNTER advances ───────────────────────────────────
fn check_rtc() -> Result<(), &'static str> {
    reg_write(RTC0_TASKS_CLEAR, 1);
    reg_write(RTC0_PRESCALER, 0); // 1:1 (writable while stopped)
    reg_write(RTC0_TASKS_START, 1);

    let c1 = reg_read(RTC0_COUNTER);
    for _ in 0..65_536 {
        if reg_read(RTC0_COUNTER) > c1 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err("rtc-not-advancing")
}

// ── i2c (TWIM): EasyDMA TX with no slave → modeled address-NACK ────────────
fn check_i2c() -> Result<(), &'static str> {
    reg_write(TWI1_ENABLE, 6); // TWIM master mode
    reg_write(TWI1_ADDRESS, 0x48);
    reg_write(TWI1_EVENTS_LASTTX, 0);
    reg_write(TWI1_EVENTS_ERROR, 0);

    let buf = core::ptr::addr_of!(I2C_TX_BUF) as u32;
    reg_write(TWI1_TXD_PTR, buf);
    reg_write(TWI1_TXD_MAXCNT, 4);
    reg_write(TWI1_TASKS_STARTTX, 1);

    // EasyDMA completes on the next bus tick; LASTTX fires either way.
    if !poll_event(TWI1_EVENTS_LASTTX) {
        return Err("i2c-no-lasttx");
    }
    // No device at ADDRESS → engine reports an address NACK and AMOUNT 0.
    if reg_read(TWI1_ERRORSRC) & ERRORSRC_ANACK == 0 {
        return Err("i2c-no-anack");
    }
    if reg_read(TWI1_EVENTS_ERROR) == 0 {
        return Err("i2c-no-error-event");
    }
    if reg_read(TWI1_TXD_AMOUNT) != 0 {
        return Err("i2c-amount");
    }
    Ok(())
}

// ── spi (SPIM2): EasyDMA TXD/RXD round-trip, EVENTS_END + AMOUNTs ──────────
fn check_spi() -> Result<(), &'static str> {
    reg_write(SPI2_ENABLE, 7); // SPIM mode
    reg_write(SPI2_EVENTS_END, 0);

    let tx = core::ptr::addr_of!(SPI_TX_BUF) as u32;
    let rx = core::ptr::addr_of!(SPI_RX_BUF) as u32;
    reg_write(SPI2_TXD_PTR, tx);
    reg_write(SPI2_TXD_MAXCNT, 4);
    reg_write(SPI2_RXD_PTR, rx);
    reg_write(SPI2_RXD_MAXCNT, 4);
    reg_write(SPI2_TASKS_START, 1);

    if !poll_event(SPI2_EVENTS_END) {
        return Err("spi-no-end");
    }
    if reg_read(SPI2_TXD_AMOUNT) != 4 {
        return Err("spi-txd-amount");
    }
    if reg_read(SPI2_RXD_AMOUNT) != 4 {
        return Err("spi-rxd-amount");
    }
    Ok(())
}

// ── adc (SAADC): real EasyDMA conversion of a fixed internal source ─────────
// The model converts explicit VDD=3.3 V against a 3.6 V full-scale, scaled to the
// configured RESOLUTION. This fixture proves a real conversion BY VALUE at two
// resolutions — it fails if the engine returned a constant or didn't convert.
fn saadc_sample(res: u32) -> Result<u16, &'static str> {
    reg_write(SAADC_ENABLE, 1); // enable SAADC
    reg_write(SAADC_RESOLUTION, res);
    reg_write(SAADC_CH0_PSELP, 9); // CH[0].PSELP = explicit internal VDD
    reg_write(SAADC_CH0_CONFIG, 0x0002_0000); // CH[0].CONFIG (gain/ref defaults)
    reg_write(SAADC_EVENTS_STARTED, 0);
    reg_write(SAADC_EVENTS_END, 0);
    reg_write(SAADC_EVENTS_RESULTDONE, 0);

    let buf = core::ptr::addr_of!(ADC_RESULT_BUF) as u32;
    reg_write(SAADC_RESULT_PTR, buf);
    reg_write(SAADC_RESULT_MAXCNT, 4);

    reg_write(SAADC_TASKS_START, 1);
    if !poll_event(SAADC_EVENTS_STARTED) {
        return Err("adc-no-started");
    }

    for _ in 0..4 {
        reg_write(SAADC_EVENTS_RESULTDONE, 0);
        reg_write(SAADC_TASKS_SAMPLE, 1);
        if !poll_event(SAADC_EVENTS_RESULTDONE) {
            return Err("adc-no-resultdone");
        }
    }
    if !poll_event(SAADC_EVENTS_END) {
        return Err("adc-no-end");
    }
    if reg_read(SAADC_EVENTS_RESULTDONE) == 0 {
        return Err("adc-no-resultdone");
    }
    if reg_read(SAADC_RESULT_AMOUNT) != 4 {
        return Err("adc-amount");
    }
    Ok(unsafe { core::ptr::read_volatile(core::ptr::addr_of!(ADC_RESULT_BUF[0])) })
}

fn check_adc() -> Result<(), &'static str> {
    // 12-bit conversion of the fixed internal source.
    let code12 = saadc_sample(2)?;
    if code12 != SAADC_CODE_12BIT {
        return Err("adc-code12");
    }
    // 10-bit conversion: the SAR core drops 2 LSBs, so the code must scale
    // down. This is what distinguishes a real conversion from a constant.
    let code10 = saadc_sample(1)?;
    if code10 != SAADC_CODE_10BIT {
        return Err("adc-code10");
    }
    if code10 >= code12 {
        return Err("adc-scale");
    }
    Ok(())
}

// ── dma: EasyDMA descriptor + payload proof (SAADC RESULT channel) ─────────
//
// nRF52833 has no central DMA controller: "DMA" here is EasyDMA, a descriptor
// engine inside UARTE/SPIM/TWIM/PWM/SAADC. The tier-1 `dma` class is declared
// for this chip by the YAML opt-in (`tier1_classes: ["dma"]`, read by
// crates/cli/src/tier1.rs); this check is the evidence behind that claim.
//
// It proves descriptor semantics that the `adc` check does not:
//   * the destination pointer is honoured — two different RAM buffers;
//   * MAXCNT is honoured — 4 then 2 samples, with sentinels proving the
//     engine did not write past MAXCNT;
//   * the payload is the converted data — every written word must equal the
//     modelled 12-bit code, so a model that only sets AMOUNT/events fails.
// The ADC conversion itself is checked separately by `adc`.
fn saadc_dma_run(ptr: u32, maxcnt: u32) -> Result<(), &'static str> {
    reg_write(SAADC_ENABLE, 1);
    reg_write(SAADC_RESOLUTION, 2); // 12-bit
    reg_write(SAADC_CH0_PSELP, 9); // explicit modeled VDD
    reg_write(SAADC_CH0_CONFIG, 0x0002_0000);
    reg_write(SAADC_EVENTS_STARTED, 0);
    reg_write(SAADC_EVENTS_END, 0);
    reg_write(SAADC_EVENTS_RESULTDONE, 0);
    reg_write(SAADC_RESULT_PTR, ptr);
    reg_write(SAADC_RESULT_MAXCNT, maxcnt);
    reg_write(SAADC_TASKS_START, 1);
    if !poll_event(SAADC_EVENTS_STARTED) {
        return Err("dma-no-started");
    }
    for _ in 0..maxcnt {
        reg_write(SAADC_EVENTS_RESULTDONE, 0);
        reg_write(SAADC_TASKS_SAMPLE, 1);
        if !poll_event(SAADC_EVENTS_RESULTDONE) {
            return Err("dma-no-resultdone");
        }
    }
    if !poll_event(SAADC_EVENTS_END) {
        return Err("dma-no-end");
    }
    if reg_read(SAADC_RESULT_AMOUNT) != maxcnt {
        return Err("dma-amount");
    }
    Ok(())
}

fn check_dma() -> Result<(), &'static str> {
    // Descriptor A: 4 samples into buffer A. Sentinel-prefill, then require
    // every word to have been replaced by the converted code.
    let a = core::ptr::addr_of_mut!(DMA_BUF_A) as *mut u16;
    for i in 0..4 {
        unsafe { core::ptr::write_volatile(a.add(i), DMA_SENTINEL) };
    }
    saadc_dma_run(core::ptr::addr_of!(DMA_BUF_A) as u32, 4)?;
    for i in 0..4 {
        if unsafe { core::ptr::read_volatile(a.add(i)) } != SAADC_CODE_12BIT {
            return Err("dma-payload-a");
        }
    }

    // Descriptor B: same engine, different pointer, shorter MAXCNT. Words
    // 0..2 must carry the payload; words 2..4 must keep the sentinel — proof
    // that MAXCNT (not just PTR) drives the transfer length.
    let b = core::ptr::addr_of_mut!(DMA_BUF_B) as *mut u16;
    for i in 0..4 {
        unsafe { core::ptr::write_volatile(b.add(i), DMA_SENTINEL) };
    }
    saadc_dma_run(core::ptr::addr_of!(DMA_BUF_B) as u32, 2)?;
    for i in 0..2 {
        if unsafe { core::ptr::read_volatile(b.add(i)) } != SAADC_CODE_12BIT {
            return Err("dma-payload-b");
        }
    }
    for i in 2..4 {
        if unsafe { core::ptr::read_volatile(b.add(i)) } != DMA_SENTINEL {
            return Err("dma-overrun");
        }
    }
    Ok(())
}

// ── wdt: configure CRV/RREN, TASKS_START, observe countdown → TIMEOUT ──────
// The model surfaces the timeout signal without resetting the core, so it is
// safe to let the dog bite here.
fn check_wdt() -> Result<(), &'static str> {
    reg_write(WDT_CRV, 64);
    reg_write(WDT_RREN, 1); // enable reload register 0
    reg_write(WDT_EVENTS_TIMEOUT, 0);
    reg_write(WDT_TASKS_START, 1);

    if reg_read(WDT_RUNSTATUS) & 1 == 0 {
        return Err("wdt-not-running");
    }
    if !poll_event(WDT_EVENTS_TIMEOUT) {
        return Err("wdt-no-timeout");
    }
    Ok(())
}

// ── pwm: configure PWM0, point SEQ[0] at a RAM duty buffer, SEQSTART0,
// observe the sequence play to SEQEND0 + PWMPERIODEND ──────────────────────
// The decoder reads the four 16-bit duty values out of PWM_SEQ_BUF by EasyDMA;
// a constant/no-op model never reaches SEQEND0, so this proves real playback.
fn check_pwm() -> Result<(), &'static str> {
    reg_write(PWM0_ENABLE, 1);
    reg_write(PWM0_MODE, 0); // Up counter
    reg_write(PWM0_PRESCALER, 0); // 16 MHz base clock
    reg_write(PWM0_COUNTERTOP, 1000);
    reg_write(PWM0_DECODER, 0); // load=Common, mode=RefreshCount
    reg_write(PWM0_LOOP, 0);
    reg_write(PWM0_PSEL_OUT0, 13); // drive P0.13 (connect bit 31 = 0)

    let seq = core::ptr::addr_of!(PWM_SEQ_BUF) as u32;
    reg_write(PWM0_SEQ0_PTR, seq);
    reg_write(PWM0_SEQ0_CNT, 4);
    reg_write(PWM0_SEQ0_REFRESH, 0);
    reg_write(PWM0_SEQ0_ENDDELAY, 0);

    reg_write(PWM0_EVENTS_SEQSTARTED0, 0);
    reg_write(PWM0_EVENTS_SEQEND0, 0);
    reg_write(PWM0_EVENTS_PWMPERIODEND, 0);

    reg_write(PWM0_TASKS_SEQSTART0, 1);

    if !poll_event(PWM0_EVENTS_SEQEND0) {
        return Err("pwm-no-seqend");
    }
    if reg_read(PWM0_EVENTS_SEQSTARTED0) == 0 {
        return Err("pwm-no-seqstarted");
    }
    if reg_read(PWM0_EVENTS_PWMPERIODEND) == 0 {
        return Err("pwm-no-periodend");
    }
    Ok(())
}

#[entry]
fn main() -> ! {
    // micro:bit v2 console bring-up: PSEL first (silicon requires a connected
    // TXD before start), 115200 baud, UARTE personality.
    reg_write(UART0_PSEL_TXD, PSEL_TXD_P0_06);
    reg_write(UART0_PSEL_RXD, PSEL_RXD_P1_08);
    reg_write(UART0_BAUDRATE, BAUDRATE_115200);
    reg_write(UART0_ENABLE, UARTE_ENABLE);

    // gpio: declared in chip YAML (gpio0 + gpio1); both ports are tested.
    report("gpio", check_gpio());

    // Behavioral peripheral round-trips against the modeled nRF52 IP.
    report("clock", check_clock());
    report("timer", check_timer());
    report("irq", check_irq());
    report("rtc", check_rtc());
    report("i2c", check_i2c());
    report("spi", check_spi());
    report("adc", check_adc());
    report("dma", check_dma());
    report("wdt", check_wdt());
    report("pwm", check_pwm());

    // uart: implicit via TIER1 done — no explicit line needed.

    uart_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
