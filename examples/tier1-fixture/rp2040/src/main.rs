//! RP2040 Tier-1 fixture firmware.
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses and reports one line per class over UART0 using the
//! TIER1 protocol:
//!
//! ```text
//! TIER1 <class> PASS
//! TIER1 <class> FAIL code=<reason>
//! TIER1 done
//! ```
//!
//! The `uart` class is implicit: receiving `TIER1 done` over UART0 is itself
//! the proof of a working UART path.
//!
//! The RP2040 chip YAML declares behavioural models for the clocks/resets
//! subsystem (`clk_rst`), the 64-bit timer, the SIO GPIO block, the PL022 SPI
//! (SPI0), the DW_apb_i2c (I2C0), the PWM slices, the ADC + temperature sensor,
//! the RTC and the watchdog. Each is exercised below with raw register
//! round-trips. The `dma` class is not attempted yet and resolves to
//! `unrecorded`.
//!
//! Register offsets follow the RP2040 datasheet: §2.14 (clocks/resets), §4.6
//! (timer), §2.3.1 (SIO GPIO), §4.3 (I2C), §4.4 (SPI), §4.2 (UART, a PL011),
//! §4.5 (PWM), §4.7 (watchdog), §4.8 (RTC), §4.9 (ADC).

#![no_std]
#![no_main]

use core::ptr::read_volatile;
use cortex_m_rt::{entry, exception};
use panic_halt as _;
use tier1_fixture_common::{rd32 as reg_read, wr32 as reg_write};

// ── UART0 (RP2040 datasheet §4.2, base 0x40034000) ────────────────────────
//
// The simulator wires uart0 with profile "pl011" (ARM PrimeCell PL011, the
// RP2040's actual UART IP). In that layout the data register (UARTDR) sits at
// offset 0x00 — writing a byte here enqueues it for transmission.
const UART0_BASE: u32 = 0x4003_4000;
const UART0_TDR: u32 = UART0_BASE;

// ── CLOCKS / RESETS (rp2040_clkrst, datasheet §2.14) ──────────────────────
//
// RESETS holds peripherals in reset out of power-on; clearing a peripheral's
// RESET bit makes the matching RESET_DONE bit assert. The block is in the
// APB/AHB window, so the RP2040 atomic CLR alias (+0x3000) is honoured by the
// bus. The PLLs report LOCK and the crystal oscillator reports STABLE.
const RESETS_BASE: u32 = 0x4000_c000;
const RESETS_RESET: u32 = RESETS_BASE;
const RESETS_RESET_CLR: u32 = RESETS_BASE + 0x3000; // atomic clear alias
const RESETS_RESET_DONE: u32 = RESETS_BASE + 0x8;
const RESETS_IO_BANK0: u32 = 1 << 5; // a representative peripheral reset bit
const PLL_SYS_CS: u32 = 0x4002_8000; // bit31 = LOCK
const XOSC_STATUS: u32 = 0x4002_4000 + 0x4; // bit31 = STABLE
const LOCK_OR_STABLE: u32 = 1 << 31;

// ── TIMER (rp2040_timer, datasheet §4.6, base 0x40054000) ─────────────────
const TIMER_BASE: u32 = 0x4005_4000;
const TIMER_TIMERAWL: u32 = TIMER_BASE + 0x28;
const TIMER_TIMERAWH: u32 = TIMER_BASE + 0x24;

// ── SIO GPIO (rp2040_sio, datasheet §2.3.1, base 0xD0000000) ──────────────
const SIO_BASE: u32 = 0xD000_0000;
const SIO_GPIO_IN: u32 = SIO_BASE + 0x04;
const SIO_GPIO_OUT: u32 = SIO_BASE + 0x10;
const SIO_GPIO_OUT_SET: u32 = SIO_BASE + 0x14;
const SIO_GPIO_OUT_CLR: u32 = SIO_BASE + 0x18;
const SIO_GPIO_OE_SET: u32 = SIO_BASE + 0x24;
const GPIO_PIN25: u32 = 1 << 25; // Pico on-board LED, safe to toggle

// ── SPI0 (rp2040_spi, PL022, datasheet §4.4, base 0x4003c000) ─────────────
const SPI0_BASE: u32 = 0x4003_c000;
const SSPCR1: u32 = SPI0_BASE + 0x04;
const SSPDR: u32 = SPI0_BASE + 0x08;
const SSPSR: u32 = SPI0_BASE + 0x0c;
const CR1_LBM: u32 = 1 << 0; // loopback
const CR1_SSE: u32 = 1 << 1; // enable
const SR_RNE: u32 = 1 << 2; // RX FIFO not empty

// ── I2C0 (rp2040_i2c, DW_apb_i2c, datasheet §4.3, base 0x40044000) ────────
const I2C0_BASE: u32 = 0x4004_4000;

// PWM (datasheet §4.5): 8 slices, 5 words each at stride 0x14, then the global
// EN / INT registers. Offsets straight from the datasheet register map.
const PWM_BASE: u32 = 0x4005_0000;
const PWM_CH0_CSR: u32 = PWM_BASE;
const PWM_CH0_DIV: u32 = PWM_BASE + 0x04;
const PWM_CH0_CTR: u32 = PWM_BASE + 0x08;
const PWM_CH0_CC: u32 = PWM_BASE + 0x0C;
const PWM_CH0_TOP: u32 = PWM_BASE + 0x10;
const PWM_CH1_CSR: u32 = PWM_BASE + 0x14;
const PWM_CH1_TOP: u32 = PWM_BASE + 0x24;
const PWM_EN: u32 = PWM_BASE + 0xA0;
const PWM_INTR: u32 = PWM_BASE + 0xA4;
const PWM_INTE: u32 = PWM_BASE + 0xA8;

// ── ADC (datasheet §4.9, base 0x4004C000) ─────────────────────────────────
const ADC_BASE: u32 = 0x4004_C000;
const ADC_CS: u32 = ADC_BASE;
const ADC_RESULT: u32 = ADC_BASE + 0x04;
const ADC_FCS: u32 = ADC_BASE + 0x08;
const ADC_FIFO: u32 = ADC_BASE + 0x0C;
const ADC_INTR: u32 = ADC_BASE + 0x14;
const ADC_CS_EN: u32 = 1 << 0;
const ADC_CS_TS_EN: u32 = 1 << 1;
const ADC_CS_START_ONCE: u32 = 1 << 2;
const ADC_CS_READY: u32 = 1 << 8;
const ADC_CS_AINSEL_SHIFT: u32 = 12;
const ADC_FCS_EN: u32 = 1 << 0;
const ADC_FCS_EMPTY: u32 = 1 << 8;
const ADC_FCS_LEVEL_SHIFT: u32 = 16;
const ADC_FCS_THRESH_SHIFT: u32 = 24;
/// The on-die temperature sensor sits on input 4 and, per the datasheet
/// relation T = 27 - (V - 0.706)/0.001721, reads 0.706 V at 27 C. On the
/// 3.3 V / 12-bit scale that is code 876.
const ADC_TEMP_INPUT: u32 = 4;
const ADC_TEMP_CODE_27C: u32 = 876;

// ── RTC (datasheet §4.8, base 0x4005C000) ─────────────────────────────────
const RTC_BASE: u32 = 0x4005_C000;
const RTC_CLKDIV_M1: u32 = RTC_BASE;
const RTC_SETUP_0: u32 = RTC_BASE + 0x04;
const RTC_SETUP_1: u32 = RTC_BASE + 0x08;
const RTC_CTRL: u32 = RTC_BASE + 0x0C;
const RTC_IRQ_SETUP_0: u32 = RTC_BASE + 0x10;
const RTC_IRQ_SETUP_1: u32 = RTC_BASE + 0x14;
const RTC_RTC_1: u32 = RTC_BASE + 0x18;
const RTC_RTC_0: u32 = RTC_BASE + 0x1C;
const RTC_INTR: u32 = RTC_BASE + 0x20;
const RTC_CTRL_ENABLE: u32 = 1 << 0;
const RTC_CTRL_ACTIVE: u32 = 1 << 1;
const RTC_CTRL_LOAD: u32 = 1 << 4;
const RTC_MATCH_ENA: u32 = 1 << 28;
const RTC_MATCH_ACTIVE: u32 = 1 << 29;
const RTC_MIN_ENA: u32 = 1 << 29;
/// 2026-07-27, Monday 12:34:56 — YEAR[23:12] MONTH[11:8] DAY[4:0] and
/// DOTW[26:24] HOUR[20:16] MIN[13:8] SEC[5:0].
const RTC_TEST_DATE: u32 = (2026 << 12) | (7 << 8) | 27;
const RTC_TEST_TIME: u32 = (1 << 24) | (12 << 16) | (34 << 8) | 56;

// ── WATCHDOG (datasheet §4.7, base 0x40058000) ────────────────────────────
const WDT_BASE: u32 = 0x4005_8000;
const WDT_CTRL: u32 = WDT_BASE;
const WDT_LOAD: u32 = WDT_BASE + 0x04;
const WDT_REASON: u32 = WDT_BASE + 0x08;
const WDT_SCRATCH0: u32 = WDT_BASE + 0x0C;
const WDT_TICK: u32 = WDT_BASE + 0x2C;
const WDT_CTRL_TIME_MASK: u32 = 0x00FF_FFFF;
const WDT_CTRL_ENABLE: u32 = 1 << 30;
const WDT_CTRL_TRIGGER: u32 = 1 << 31;
const WDT_REASON_TIMER: u32 = 1 << 0;
const WDT_REASON_FORCE: u32 = 1 << 1;
const WDT_TICK_ENABLE: u32 = 1 << 9;
const WDT_TICK_RUNNING: u32 = 1 << 10;
/// CTRL resets with all three PAUSE_* bits set; TICK resets with ENABLE set
/// and CYCLES zero (so RUNNING is clear). Both are SVD reset values.
const WDT_CTRL_RESET: u32 = 0x0700_0000;
const WDT_TICK_RESET: u32 = 0x0000_0200;

// ── NVIC (ARMv6-M, SCS) ───────────────────────────────────────────────────
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ICPR0: u32 = 0xE000_E280;
/// PWM_IRQ_WRAP — RP2040 datasheet §2.3.2, and the SVD's PWM interrupt entry.
const PWM_IRQ_WRAP: i16 = 4;

const IC_DATA_CMD: u32 = I2C0_BASE + 0x10;
const IC_RAW_INTR_STAT: u32 = I2C0_BASE + 0x34;
const IC_ENABLE: u32 = I2C0_BASE + 0x6c;
const IC_TX_ABRT_SOURCE: u32 = I2C0_BASE + 0x80;
const INTR_TX_ABRT: u32 = 1 << 6;
const ABRT_7B_ADDR_NOACK: u32 = 1 << 0;

// ── UART0 output (raw register writes) ───────────────────────────────────
fn uart_write_byte(byte: u8) {
    reg_write(UART0_TDR, byte as u32);
}

fn uart_write_str(s: &str) {
    for b in s.as_bytes() {
        uart_write_byte(*b);
    }
}

fn uart_write_line(s: &str) {
    uart_write_str(s);
    uart_write_str("\r\n");
}

// ── pwm: a slice counts on its divided clock, wraps at TOP and latches INTR ──
fn check_pwm() -> Result<(), &'static str> {
    // Reset values first: TOP all-ones and DIV = 1.0 (datasheet §4.5.3).
    if reg_read(PWM_CH0_TOP) != 0xFFFF {
        return Err("pwm-top-reset");
    }
    if reg_read(PWM_CH0_DIV) != 0x010 {
        return Err("pwm-div-reset");
    }

    // A disabled slice must not count.
    let idle = reg_read(PWM_CH0_CTR);
    for _ in 0..64 {
        core::hint::spin_loop();
    }
    if reg_read(PWM_CH0_CTR) != idle {
        return Err("pwm-counts-while-disabled");
    }

    // Compare + wrap values must round-trip so duty-cycle maths is meaningful.
    reg_write(PWM_CH0_CC, 0x0004_0008);
    if reg_read(PWM_CH0_CC) != 0x0004_0008 {
        return Err("pwm-cc-readback");
    }

    // Arm slice 0 with a short wrap, enable via the EN alias, and require both
    // a moving counter and a latched wrap interrupt.
    reg_write(PWM_CH0_TOP, 0x000F);
    reg_write(PWM_INTR, 0xFF);
    reg_write(PWM_EN, 1 << 0);
    if reg_read(PWM_CH0_CSR) & 1 == 0 {
        return Err("pwm-en-alias");
    }
    for _ in 0..512 {
        core::hint::spin_loop();
    }
    if reg_read(PWM_CH0_CTR) > 0x000F {
        return Err("pwm-ctr-above-top");
    }
    if reg_read(PWM_INTR) & 1 == 0 {
        return Err("pwm-no-wrap");
    }

    // Slice 1 was never enabled, so it must be untouched — proving the slices
    // are independent rather than one shared counter.
    if reg_read(PWM_CH1_CSR) & 1 != 0 {
        return Err("pwm-slice-bleed");
    }

    reg_write(PWM_EN, 0);
    Ok(())
}

// ── adc: 12-bit SAR, temperature sensor and sample FIFO ───────────────────
fn check_adc() -> Result<(), &'static str> {
    // CS resets to all-zero, so READY is clear until the converter is powered.
    if reg_read(ADC_CS) != 0 {
        return Err("adc-cs-reset");
    }
    reg_write(ADC_CS, ADC_CS_EN);
    if reg_read(ADC_CS) & ADC_CS_READY == 0 {
        return Err("adc-not-ready");
    }

    // Powered, the temperature sensor converts the datasheet transfer function
    // T = 27 - (V - 0.706)/0.001721, i.e. 0.706 V (code 876) at 27 C.
    let select_ts = ADC_CS_EN | (ADC_TEMP_INPUT << ADC_CS_AINSEL_SHIFT);
    reg_write(ADC_CS, select_ts | ADC_CS_TS_EN | ADC_CS_START_ONCE);
    if reg_read(ADC_RESULT) != ADC_TEMP_CODE_27C {
        return Err("adc-ts-code");
    }

    // With TS_EN clear the sensor is unpowered, and an unpowered sensor must
    // stop reporting a temperature rather than keep the old code alive.
    reg_write(ADC_CS, select_ts | ADC_CS_START_ONCE);
    if reg_read(ADC_RESULT) != 0 {
        return Err("adc-ts-unpowered");
    }

    // FIFO: enabled, it collects conversions; LEVEL tracks occupancy and a
    // drained FIFO reads EMPTY again.
    reg_write(ADC_FCS, ADC_FCS_EN | (2 << ADC_FCS_THRESH_SHIFT));
    if reg_read(ADC_FCS) & ADC_FCS_EMPTY == 0 {
        return Err("adc-fifo-not-empty");
    }
    reg_write(ADC_CS, select_ts | ADC_CS_TS_EN | ADC_CS_START_ONCE);
    reg_write(ADC_CS, select_ts | ADC_CS_TS_EN | ADC_CS_START_ONCE);
    if (reg_read(ADC_FCS) >> ADC_FCS_LEVEL_SHIFT) & 0xF != 2 {
        return Err("adc-fifo-level");
    }
    // The threshold is 2, so the raw FIFO interrupt is a level that is now set.
    if reg_read(ADC_INTR) & 1 == 0 {
        return Err("adc-intr-level");
    }
    if reg_read(ADC_FIFO) != ADC_TEMP_CODE_27C {
        return Err("adc-fifo-data");
    }
    reg_read(ADC_FIFO);
    if reg_read(ADC_FCS) & ADC_FCS_EMPTY == 0 {
        return Err("adc-fifo-not-drained");
    }
    // Draining below the threshold drops the level — nothing to acknowledge.
    if reg_read(ADC_INTR) & 1 != 0 {
        return Err("adc-intr-stuck");
    }

    reg_write(ADC_FCS, 0);
    reg_write(ADC_CS, 0);
    Ok(())
}

// ── rtc: calendar, the RTC_1-before-RTC_0 read latch, and the alarm ────────
fn check_rtc() -> Result<(), &'static str> {
    if reg_read(RTC_CTRL) != 0 {
        return Err("rtc-ctrl-reset");
    }

    // Stage a date/time and commit it with CTRL.LOAD, then start the clock.
    // A 16-tick second keeps the fixture short (silicon uses 46875) while still
    // being slow enough that a polling loop cannot step over a whole second.
    reg_write(RTC_CTRL, 0);
    reg_write(RTC_CLKDIV_M1, 15);
    reg_write(RTC_SETUP_0, RTC_TEST_DATE);
    reg_write(RTC_SETUP_1, RTC_TEST_TIME);
    reg_write(RTC_CTRL, RTC_CTRL_LOAD);

    // Read in the datasheet's order: RTC_1 latches the pair, RTC_0 reads it.
    if reg_read(RTC_RTC_1) != RTC_TEST_DATE {
        return Err("rtc-date-load");
    }
    if reg_read(RTC_RTC_0) != RTC_TEST_TIME {
        return Err("rtc-time-load");
    }

    reg_write(RTC_CTRL, RTC_CTRL_ENABLE);
    if reg_read(RTC_CTRL) & RTC_CTRL_ACTIVE == 0 {
        return Err("rtc-not-active");
    }

    // The clock runs: a fresh latch eventually reports a different time.
    let mut spins = 0u32;
    let latched = loop {
        reg_read(RTC_RTC_1);
        let t = reg_read(RTC_RTC_0);
        if t != RTC_TEST_TIME {
            break t;
        }
        spins += 1;
        if spins > 100_000 {
            return Err("rtc-not-running");
        }
    };

    // The latch is real: without re-reading RTC_1, RTC_0 keeps returning the
    // value captured above however much time passes.
    for _ in 0..256 {
        core::hint::spin_loop();
    }
    if reg_read(RTC_RTC_0) != latched {
        return Err("rtc-latch-not-held");
    }
    reg_read(RTC_RTC_1);
    if reg_read(RTC_RTC_0) == latched {
        return Err("rtc-latch-not-refreshed");
    }

    // Alarm. Stop and reload the clock first so the match is exact rather than
    // a race against a running calendar, then match on the minute field alone:
    // MATCH_ENA gates the whole comparison and the unenabled fields are ignored.
    reg_write(RTC_CTRL, 0);
    reg_write(RTC_SETUP_1, RTC_TEST_TIME); // 12:34:56
    reg_write(RTC_CTRL, RTC_CTRL_LOAD);

    reg_write(RTC_IRQ_SETUP_1, RTC_MIN_ENA | (34 << 8));
    reg_write(RTC_IRQ_SETUP_0, 0);
    if reg_read(RTC_INTR) & 1 != 0 {
        return Err("rtc-alarm-ungated");
    }
    reg_write(RTC_IRQ_SETUP_0, RTC_MATCH_ENA);
    if reg_read(RTC_INTR) & 1 == 0 {
        return Err("rtc-alarm-no-match");
    }
    if reg_read(RTC_IRQ_SETUP_0) & RTC_MATCH_ACTIVE == 0 {
        return Err("rtc-match-active");
    }

    // Re-target the next minute: the match must drop, then come back on its own
    // once the running calendar reaches it.
    reg_write(RTC_IRQ_SETUP_1, RTC_MIN_ENA | (35 << 8));
    if reg_read(RTC_INTR) & 1 != 0 {
        return Err("rtc-alarm-stale-match");
    }
    reg_write(RTC_CTRL, RTC_CTRL_ENABLE);
    let mut spins = 0u32;
    while reg_read(RTC_INTR) & 1 == 0 {
        spins += 1;
        if spins > 200_000 {
            return Err("rtc-alarm-never-fires");
        }
    }

    // Disabling the alarm is the acknowledgement — there is no raw-status write.
    reg_write(RTC_IRQ_SETUP_0, 0);
    if reg_read(RTC_INTR) & 1 != 0 {
        return Err("rtc-alarm-stuck");
    }
    reg_write(RTC_CTRL, 0);
    Ok(())
}

// ── wdt: tick generator, the RP2040-E1 double decrement, and REASON ───────
fn check_wdt() -> Result<(), &'static str> {
    if reg_read(WDT_CTRL) != WDT_CTRL_RESET {
        return Err("wdt-ctrl-reset");
    }
    // ENABLE is set out of reset but CYCLES is zero, so the generator is NOT
    // running — which is exactly why TICK reads 0x200 and not 0x600.
    if reg_read(WDT_TICK) != WDT_TICK_RESET {
        return Err("wdt-tick-reset");
    }
    if reg_read(WDT_REASON) != 0 {
        return Err("wdt-reason-reset");
    }

    // Scratch registers are plain storage the bootrom reboot path relies on.
    reg_write(WDT_SCRATCH0, 0xC0FF_EE00);
    if reg_read(WDT_SCRATCH0) != 0xC0FF_EE00 {
        return Err("wdt-scratch");
    }

    // Start the tick generator (1:1) and load a large, even count.
    reg_write(WDT_TICK, 1 | WDT_TICK_ENABLE);
    if reg_read(WDT_TICK) & WDT_TICK_RUNNING == 0 {
        return Err("wdt-tick-not-running");
    }
    reg_write(WDT_LOAD, 0x00FF_FFFE);
    if reg_read(WDT_LOAD) != 0 {
        return Err("wdt-load-not-write-only");
    }
    if reg_read(WDT_CTRL) & WDT_CTRL_TIME_MASK != 0x00FF_FFFE {
        return Err("wdt-load");
    }

    // Enabled, the counter falls.
    reg_write(WDT_CTRL, WDT_CTRL_RESET | WDT_CTRL_ENABLE);
    let mut spins = 0u32;
    while reg_read(WDT_CTRL) & WDT_CTRL_TIME_MASK == 0x00FF_FFFE {
        spins += 1;
        if spins > 100_000 {
            return Err("wdt-not-counting");
        }
    }

    // Errata RP2040-E1: two counts per tick. Started from an even value the
    // countdown can therefore never be observed odd — a one-per-tick counter
    // would be caught within a couple of samples.
    for _ in 0..16 {
        if reg_read(WDT_CTRL) & 1 != 0 {
            return Err("wdt-single-decrement");
        }
        for _ in 0..8 {
            core::hint::spin_loop();
        }
    }

    // Feeding reloads the counter (it keeps falling, so compare against the
    // value immediately before the feed rather than against LOAD).
    let before_feed = reg_read(WDT_CTRL) & WDT_CTRL_TIME_MASK;
    reg_write(WDT_LOAD, 0x00FF_FFFE);
    if reg_read(WDT_CTRL) & WDT_CTRL_TIME_MASK <= before_feed {
        return Err("wdt-feed");
    }

    // Let a short load expire: REASON.TIMER latches and the counter floors.
    reg_write(WDT_LOAD, 0x40);
    let mut spins = 0u32;
    while reg_read(WDT_REASON) & WDT_REASON_TIMER == 0 {
        spins += 1;
        if spins > 100_000 {
            return Err("wdt-no-timeout");
        }
    }
    if reg_read(WDT_CTRL) & WDT_CTRL_TIME_MASK != 0 {
        return Err("wdt-time-after-timeout");
    }

    // CTRL.TRIGGER forces a bite and is write-only.
    reg_write(WDT_CTRL, WDT_CTRL_RESET | WDT_CTRL_TRIGGER);
    if reg_read(WDT_REASON) & WDT_REASON_FORCE == 0 {
        return Err("wdt-no-force");
    }
    if reg_read(WDT_CTRL) & WDT_CTRL_TRIGGER != 0 {
        return Err("wdt-trigger-readback");
    }

    reg_write(WDT_CTRL, WDT_CTRL_RESET);
    Ok(())
}

static mut IRQ_HITS: u32 = 0;

/// irq: a real peripheral source drives a real NVIC vector. Arm PWM slice 1
/// with a short wrap, unmask its INTE and PWM_IRQ_WRAP in the NVIC, and require
/// the vector to actually run. The handler silences the (level-held) source and
/// disarms itself so the main thread cannot be wedged by a re-pend.
fn check_irq() -> Result<(), &'static str> {
    reg_write(PWM_EN, 0);
    reg_write(PWM_INTR, 0xFF);
    reg_write(PWM_CH1_TOP, 0x000F);
    reg_write(PWM_INTE, 1 << 1);
    reg_write(NVIC_ICPR0, 1 << PWM_IRQ_WRAP as u32);
    reg_write(NVIC_ISER0, 1 << PWM_IRQ_WRAP as u32);
    reg_write(PWM_EN, 1 << 1);

    for _ in 0..100_000 {
        if unsafe { read_volatile(core::ptr::addr_of!(IRQ_HITS)) } != 0 {
            return Ok(());
        }
    }
    reg_write(PWM_EN, 0);
    reg_write(PWM_INTE, 0);
    reg_write(NVIC_ICER0, 1 << PWM_IRQ_WRAP as u32);
    Err("irq-not-delivered")
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == PWM_IRQ_WRAP {
        // Silence the source first: PWM_IRQ_WRAP is level-held while a masked
        // wrap is pending, so leaving it asserted would re-enter forever.
        reg_write(PWM_EN, 0);
        reg_write(PWM_INTE, 0);
        reg_write(PWM_INTR, 0xFF);
        reg_write(NVIC_ICER0, 1 << PWM_IRQ_WRAP as u32);
        reg_write(NVIC_ICPR0, 1 << PWM_IRQ_WRAP as u32);
        unsafe {
            let p = core::ptr::addr_of_mut!(IRQ_HITS);
            core::ptr::write_volatile(p, read_volatile(p) + 1);
        }
    }
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

// ── clock: clear a RESET bit → RESET_DONE asserts; PLL LOCK + XOSC STABLE ──
fn check_clock() -> Result<(), &'static str> {
    // Out of reset IO_BANK0 is held in reset, so its RESET_DONE bit is 0.
    if reg_read(RESETS_RESET) & RESETS_IO_BANK0 == 0 {
        return Err("reset-not-asserted");
    }
    // Release it via the atomic CLR alias; RESET_DONE must then reflect it.
    reg_write(RESETS_RESET_CLR, RESETS_IO_BANK0);
    if reg_read(RESETS_RESET) & RESETS_IO_BANK0 != 0 {
        return Err("reset-not-cleared");
    }
    if reg_read(RESETS_RESET_DONE) & RESETS_IO_BANK0 == 0 {
        return Err("reset-done");
    }
    // PLL_SYS reports LOCK and the crystal oscillator reports STABLE.
    if reg_read(PLL_SYS_CS) & LOCK_OR_STABLE == 0 {
        return Err("pll-lock");
    }
    if reg_read(XOSC_STATUS) & LOCK_OR_STABLE == 0 {
        return Err("xosc-stable");
    }
    Ok(())
}

// ── timer: the free-running 64-bit counter advances ────────────────────────
fn check_timer() -> Result<(), &'static str> {
    let a = reg_read(TIMER_TIMERAWL);
    for _ in 0..256 {
        core::hint::spin_loop();
    }
    let b = reg_read(TIMER_TIMERAWL);
    if b == a {
        return Err("timer-not-advancing");
    }
    // The high word must be a sane (small) value, proving the 64-bit split is
    // wired rather than aliasing the low word.
    if reg_read(TIMER_TIMERAWH) > 1 {
        return Err("timer-high-word");
    }
    Ok(())
}

// ── gpio: SIO drive + readback round-trip on GPIO25 ───────────────────────
fn check_gpio() -> Result<(), &'static str> {
    // Enable the output driver, then set the pin high and read it back.
    reg_write(SIO_GPIO_OE_SET, GPIO_PIN25);
    reg_write(SIO_GPIO_OUT_SET, GPIO_PIN25);
    if reg_read(SIO_GPIO_OUT) & GPIO_PIN25 == 0 {
        return Err("gpio-out-set");
    }
    if reg_read(SIO_GPIO_IN) & GPIO_PIN25 == 0 {
        return Err("gpio-in-high");
    }
    // Clear it; the input must follow.
    reg_write(SIO_GPIO_OUT_CLR, GPIO_PIN25);
    if reg_read(SIO_GPIO_IN) & GPIO_PIN25 != 0 {
        return Err("gpio-in-low");
    }
    Ok(())
}

// ── spi: PL022 internal-loopback transfer round-trips a byte ──────────────
fn check_spi() -> Result<(), &'static str> {
    // Enable the SSP in internal-loopback mode (MOSI wired to MISO).
    reg_write(SSPCR1, CR1_SSE | CR1_LBM);
    reg_write(SSPDR, 0xA5);
    // The byte clocks straight into the RX FIFO; RNE must assert.
    let mut spins = 0u32;
    while reg_read(SSPSR) & SR_RNE == 0 {
        spins += 1;
        if spins > 100_000 {
            return Err("spi-no-rx");
        }
    }
    if reg_read(SSPDR) != 0xA5 {
        return Err("spi-data");
    }
    Ok(())
}

// ── i2c: master transfer to an unconnected target → address-NACK abort ────
fn check_i2c() -> Result<(), &'static str> {
    reg_write(IC_ENABLE, 1);
    reg_write(IC_DATA_CMD, 0xDE); // write one byte to a 7-bit target
    if reg_read(IC_RAW_INTR_STAT) & INTR_TX_ABRT == 0 {
        return Err("i2c-no-abort");
    }
    if reg_read(IC_TX_ABRT_SOURCE) & ABRT_7B_ADDR_NOACK == 0 {
        return Err("i2c-abort-source");
    }
    Ok(())
}

#[entry]
fn main() -> ! {
    // Behavioural peripheral round-trips against the modeled RP2040.
    report("clock", check_clock());
    report("timer", check_timer());
    report("gpio", check_gpio());
    report("spi", check_spi());
    report("i2c", check_i2c());
    report("pwm", check_pwm());
    report("adc", check_adc());
    report("rtc", check_rtc());
    report("wdt", check_wdt());
    // irq last: it leaves the NVIC unmasked only for as long as the check runs.
    report("irq", check_irq());

    // uart: implicit via TIER1 done — no explicit line needed.
    uart_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
