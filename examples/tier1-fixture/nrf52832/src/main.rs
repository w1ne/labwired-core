//! nRF52832 Tier-1 fixture firmware.
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
//! The nRF52832 chip YAML declares `uart0`, `gpio0` (single P0 port), `clock`,
//! `timer0`, `twi1`, `spi2`, `rtc0`, and `wdt`. nRF52832 is near-identical
//! silicon to nRF52840 and shares the nRF52 series peripheral memory map, so
//! these checks mirror the proven nrf52840 fixture. Classes with no declared
//! peripheral (e.g. dma, irq) resolve to `na` by the parser.
//!
//! Register offsets follow the nRF52832 Product Specification v1.4
//! (compatible with nRF52840 PS v1.7 for the shared peripheral layouts).

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// ── UART0 (nRF52832 PS §15.8 memory map, base 0x40002000) ─────────────────
//
// The nrf52832 chip YAML models uart0 with the nRF52 generic UART profile,
// which captures a 32-bit write to TXD (offset 0x51C) as one TX byte. This is
// the legacy non-DMA path and is exactly what the existing fixture used to
// emit `TIER1 done`; it is reused here unchanged.
const UART0_BASE: u32 = 0x4000_2000;
const UART0_ENABLE: u32 = UART0_BASE + 0x500;
const UART0_TXD: u32 = UART0_BASE + 0x51C;

// ── GPIO0 / P0 (base 0x50000000) ──────────────────────────────────────────
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

// ── CLOCK (nrf_clock, base 0x40000000) ────────────────────────────────────
const CLOCK_BASE: u32 = 0x4000_0000;
const CLOCK_TASKS_HFCLKSTART: u32 = CLOCK_BASE;
const CLOCK_EVENTS_HFCLKSTARTED: u32 = CLOCK_BASE + 0x100;
const CLOCK_HFCLKRUN: u32 = CLOCK_BASE + 0x408;

// ── TIMER0 (nrf52840_timer, base 0x40008000) ──────────────────────────────
const TIMER0_BASE: u32 = 0x4000_8000;
const TIMER0_TASKS_START: u32 = TIMER0_BASE;
const TIMER0_TASKS_CLEAR: u32 = TIMER0_BASE + 0x00C;
const TIMER0_TASKS_CAPTURE0: u32 = TIMER0_BASE + 0x040;
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
// RESULT.MAXCNT samples to RESULT.PTR and fires END + RESULTDONE.
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
// Converted codes for the model's fixed internal source (V(P)=3.0 V, 3.6 V
// full-scale): code(N) = (3.0/3.6) * 2^N, narrower resolutions drop LSBs.
const SAADC_CODE_12BIT: u16 = 3413; // (3.0/3.6) * 2^12
const SAADC_CODE_10BIT: u16 = 853; // (3.0/3.6) * 2^10

// ── WDT (nrf52840_watchdog, base 0x40010000) ──────────────────────────────
const WDT_BASE: u32 = 0x4001_0000;
const WDT_TASKS_START: u32 = WDT_BASE;
const WDT_EVENTS_TIMEOUT: u32 = WDT_BASE + 0x100;
const WDT_RUNSTATUS: u32 = WDT_BASE + 0x400;
const WDT_CRV: u32 = WDT_BASE + 0x504;
const WDT_RREN: u32 = WDT_BASE + 0x508;

// PWM0 (nRF52832 PS §47, base 0x4001C000; same model as nRF52840). The
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

#[inline(always)]
fn reg_read(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
fn reg_write(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

// EasyDMA buffers for the TWIM (I2C) and SPIM (SPI) checks. Static .bss RAM,
// the only region the EasyDMA engines can address.
static mut I2C_TX_BUF: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
static mut SPI_TX_BUF: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
static mut SPI_RX_BUF: [u8; 4] = [0; 4];

// SAADC RESULT buffer (4 x 16-bit samples). Static .bss RAM, the only region
// the EasyDMA engine can address.
static mut ADC_RESULT_BUF: [u16; 4] = [0; 4];

// PWM SEQ[0] duty buffer (4 x 16-bit). Static .bss RAM — the sequence engine
// reads these duty values out by EasyDMA at SEQ[0].PTR.
static mut PWM_SEQ_BUF: [u16; 4] = [0x8000 | 250, 0x8000 | 500, 0x8000 | 750, 0x8000 | 1000];

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

// ── UART0 output (legacy byte-at-a-time TXD writes) ──────────────────────
//
// Write each byte directly to the TXD register. The simulator's nRF52 UART
// model captures each word write as one TX byte (low 8 bits).
fn uart_write_byte(byte: u8) {
    reg_write(UART0_TXD, byte as u32);
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

// ── gpio: DIRSET + OUTSET/OUTCLR on P0.13, read back via OUT ─────────────
fn check_gpio() -> Result<(), &'static str> {
    const PIN: u32 = 1 << 13;

    // Configure pin as output.
    reg_write(GPIO0_DIRSET, PIN);

    // Set pin high via OUTSET, read back via OUT.
    reg_write(GPIO0_OUTSET, PIN);
    if reg_read(GPIO0_OUT) & PIN == 0 {
        return Err("gpio-out-outset");
    }

    // Clear pin via OUTCLR, read back via OUT.
    reg_write(GPIO0_OUTCLR, PIN);
    if reg_read(GPIO0_OUT) & PIN != 0 {
        return Err("gpio-out-outclr");
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

// ── spi (SPIM): EasyDMA TXD/RXD round-trip, EVENTS_END + AMOUNTs ───────────
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
// The model converts V(P)=3.0 V against a 3.6 V full-scale, scaled to the
// configured RESOLUTION. This fixture proves a real conversion BY VALUE at two
// resolutions — it fails if the engine returned a constant or didn't convert.
fn saadc_sample(res: u32) -> Result<u16, &'static str> {
    reg_write(SAADC_ENABLE, 1); // enable SAADC
    reg_write(SAADC_RESOLUTION, res);
    reg_write(SAADC_CH0_PSELP, 1); // CH[0].PSELP = AnalogInput0
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

    reg_write(SAADC_TASKS_SAMPLE, 1);
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
    // Enable UART0 (value 4 per Nordic PS UART.ENABLE field).
    reg_write(UART0_ENABLE, 4);

    // gpio: declared in chip YAML (gpio0 / P0).
    report("gpio", check_gpio());

    // Behavioral peripheral round-trips against the modeled nRF52 IP.
    report("clock", check_clock());
    report("timer", check_timer());
    report("rtc", check_rtc());
    report("i2c", check_i2c());
    report("spi", check_spi());
    report("adc", check_adc());
    report("wdt", check_wdt());
    report("pwm", check_pwm());

    // uart: implicit via TIER1 done — no explicit line needed.

    uart_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
