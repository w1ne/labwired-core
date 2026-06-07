//! ESP32-classic Tier-1 fixture firmware.
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses (no esp-hal drivers — esp-hal is used only for init /
//! entry / println scaffolding) and reports one line per peripheral class
//! over UART0 using the TIER1 protocol:
//!
//! ```text
//! TIER1 <class> PASS
//! TIER1 <class> FAIL code=<reason>
//! TIER1 done
//! ```
//!
//! The `uart` class is implicit: receiving `TIER1 done` over UART0 is itself
//! the proof of a working UART path, so no `uart` line is ever printed.
//!
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets and bit
//! positions follow the ESP32 TRM v4.6 and are cross-checked against the
//! simulator's model sources (`crates/core/src/peripherals/esp32/`).
//!
//! # UART note
//! The sim registers UART0 at 0x3FF4_0000 with the STM32F1 register layout
//! (DR at 0x00 transmits, STATUS at 0x1C returns 0xC0 for TX-ready). The
//! real ESP32 UART FIFO is at the same offset 0x00, so the byte-push path
//! works identically in sim and on silicon. The STATUS poll uses bits[25:16]
//! (TXFIFO_CNT on real silicon); in the sim those bits are always 0 (< 128)
//! so the poll exits immediately — correct behaviour, just instant.
//!
//! # IRQ note
//! On ESP32-classic there is no separate INTMATRIX peripheral (that was
//! introduced on S3). The interrupt-matrix MAP registers live inside DPORT
//! (0x3FF0_0000, TRM §9): PRO_CPU source `s` maps at
//! `DPORT_BASE + 0x104 + s*4`. The DPORT model round-trips all writes, so
//! a map-register write-then-read-back proves the wiring is observable.

#![no_std]
#![no_main]

use esp_backtrace as _;

// ── Peripheral base addresses (ESP32 TRM v4.6 §1.3 memory map) ─────────────
//
// UART0  at 0x3FF4_0000 — DR/FIFO at offset 0x00, STATUS at offset 0x1C
// GPIO   at 0x3FF4_4000 — OUT/W1TS/W1TC/ENABLE/W1TS/W1TC at standard offsets
// TIMG0  at 0x3FF5_F000 — T0CONFIG/T0LO/T0HI/T0UPDATE at standard offsets
// DPORT  at 0x3FF0_0000 — interrupt-matrix MAP registers at 0x104+src*4

const UART0_BASE: u32 = 0x3FF4_0000;
const GPIO_BASE: u32 = 0x3FF4_4000;
const TIMG0_BASE: u32 = 0x3FF5_F000;
const DPORT_BASE: u32 = 0x3FF0_0000;

#[inline(always)]
fn reg_read(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
fn reg_write(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

/// Fixed-iteration busy spin. Deterministic in the simulator.
fn spin(iters: u32) {
    for i in 0..iters {
        core::hint::black_box(i);
    }
}

// ── UART0 protocol output ─────────────────────────────────────────────────
//
// DR (TX data register) at offset 0x00. On the sim this uses the STM32F1
// layout so a write to 0x00 transmits the byte. On real ESP32 silicon the
// UART FIFO is also mapped at offset 0x00, so the path is correct on both.
//
// STATUS at offset 0x1C — on real silicon bits[25:16] = TXFIFO_CNT. Bounded
// poll: if FIFO is "full" (≥128 slots used) we wait; in the sim STATUS is
// 0xC0 so TXFIFO_CNT=0 and the poll exits immediately.
fn uart0_write_byte(byte: u8) {
    const FIFO: u32 = UART0_BASE + 0x00;
    const STATUS: u32 = UART0_BASE + 0x1C;
    const TXFIFO_LEN: u32 = 128;

    // Bounded poll for TX-FIFO space.
    for _ in 0..1_000_000 {
        if ((reg_read(STATUS) >> 16) & 0x3FF) < TXFIFO_LEN {
            break;
        }
    }
    reg_write(FIFO, byte as u32);
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

// ── clock: TIMG0 T0 counter advances between two latched reads ────────────
//
// TRM §16 (TIMG0 at 0x3FF5_F000):
//   T0CONFIG  @ 0x00 — bit 31 = EN, bit 30 = INCREASE, bits[28:13] = DIVIDER
//   T0LO      @ 0x04 — latched low 32 bits (after T0UPDATE strobe)
//   T0HI      @ 0x08 — latched high 32 bits
//   T0UPDATE  @ 0x0C — write any value to latch live counter into LO/HI
//
// The model (`crates/core/src/peripherals/esp32/timg.rs`) ticks T0 once per
// sim tick when T0CONFIG.EN is set. Setting DIVIDER=1 (bits[28:13]=1,
// i.e. value 1<<13) selects divide-by-1 and maximises tick rate. Two UPDATE
// latches with a spin between them must produce a strictly increasing value.
fn check_clock() -> Result<(), &'static str> {
    const T0CONFIG: u32 = TIMG0_BASE + 0x00;
    const T0LO: u32 = TIMG0_BASE + 0x04;
    const T0HI: u32 = TIMG0_BASE + 0x08;
    const T0UPDATE: u32 = TIMG0_BASE + 0x0C;
    const EN: u32 = 1 << 31;
    const INCREASE: u32 = 1 << 30;
    const DIVIDER_1: u32 = 1 << 13;

    reg_write(T0CONFIG, EN | INCREASE | DIVIDER_1);

    let latch = || -> u64 {
        reg_write(T0UPDATE, 1);
        ((reg_read(T0HI) as u64) << 32) | reg_read(T0LO) as u64
    };

    let t1 = latch();
    spin(20_000);
    let t2 = latch();
    if t2 > t1 {
        Ok(())
    } else {
        Err("timg0-not-advancing")
    }
}

// ── gpio: ENABLE_W1TS + OUT_W1TS/W1TC on GPIO4, read back via OUT ─────────
//
// TRM §4.10 (GPIO at 0x3FF4_4000):
//   OUT        @ 0x04 — current output values GPIO0..31
//   OUT_W1TS   @ 0x08 — write 1 to set bit
//   OUT_W1TC   @ 0x0C — write 1 to clear bit
//   ENABLE     @ 0x20 — output enable GPIO0..31
//   ENABLE_W1TS@ 0x24 — write 1 to enable bit
//   ENABLE_W1TC@ 0x28 — write 1 to disable bit
//
// GPIO4 carries no boot strap on the standard WROOM-32 module.
fn check_gpio() -> Result<(), &'static str> {
    const OUT: u32 = GPIO_BASE + 0x04;
    const OUT_W1TS: u32 = GPIO_BASE + 0x08;
    const OUT_W1TC: u32 = GPIO_BASE + 0x0C;
    const ENABLE: u32 = GPIO_BASE + 0x20;
    const ENABLE_W1TS: u32 = GPIO_BASE + 0x24;
    const ENABLE_W1TC: u32 = GPIO_BASE + 0x28;
    const PIN: u32 = 1 << 4;

    reg_write(ENABLE_W1TS, PIN);
    if reg_read(ENABLE) & PIN == 0 {
        return Err("gpio-enable-w1ts");
    }
    reg_write(OUT_W1TS, PIN);
    if reg_read(OUT) & PIN == 0 {
        return Err("gpio-out-w1ts");
    }
    reg_write(OUT_W1TC, PIN);
    if reg_read(OUT) & PIN != 0 {
        return Err("gpio-out-w1tc");
    }
    reg_write(ENABLE_W1TC, PIN);
    if reg_read(ENABLE) & PIN != 0 {
        return Err("gpio-enable-w1tc");
    }
    Ok(())
}

// ── timer: TIMG0 T0 enabled + increase, two T0UPDATE latches differ ───────
//
// Same peripheral as check_clock() above; this class proves timer-counter
// semantics independently. T0 should already be running from check_clock()
// but we re-arm it here to be self-contained.
fn check_timer() -> Result<(), &'static str> {
    const T0CONFIG: u32 = TIMG0_BASE + 0x00;
    const T0LO: u32 = TIMG0_BASE + 0x04;
    const T0HI: u32 = TIMG0_BASE + 0x08;
    const T0UPDATE: u32 = TIMG0_BASE + 0x0C;
    const EN: u32 = 1 << 31;
    const INCREASE: u32 = 1 << 30;
    const DIVIDER_1: u32 = 1 << 13;

    reg_write(T0CONFIG, EN | INCREASE | DIVIDER_1);

    let latch = || -> u64 {
        reg_write(T0UPDATE, 1);
        ((reg_read(T0HI) as u64) << 32) | reg_read(T0LO) as u64
    };
    let a = latch();
    spin(20_000);
    let b = latch();
    if b > a {
        Ok(())
    } else {
        Err("timg0-not-counting")
    }
}

// ── irq: DPORT interrupt-matrix MAP register write + read-back ────────────
//
// TRM §9.4 (DPORT at 0x3FF0_0000, interrupt matrix §7):
//   PRO_CPU interrupt-matrix base: DPORT_BASE + 0x104 (DPORT_PRO_MAC_INTR_MAP)
//   Source `s` maps to CPU slot via: DPORT_BASE + 0x104 + s * 4
//
// We use source 10 (arbitrary, well within the 128-source matrix, offset
// 0x104 + 10*4 = 0x12C — within the 4 KiB DPORT window at 0x3FF0_0000).
// Writing a CPU-slot value (e.g. 17) and reading it back proves the DPORT
// model's MAP register storage round-trips. No interrupt is actually
// delivered — INT_ENA = 0 keeps all sources inert, and delivery is covered
// by dedicated intmatrix tests in crates/core/tests/.
fn check_irq() -> Result<(), &'static str> {
    // Source 10 PRO_CPU MAP register: DPORT_BASE + 0x104 + 10*4 = 0x12C offset
    const PRO_MAP_BASE: u32 = DPORT_BASE + 0x0104;
    const SOURCE: u32 = 10;
    const MAP_REG: u32 = PRO_MAP_BASE + 4 * SOURCE;

    reg_write(MAP_REG, 17);
    if reg_read(MAP_REG) != 17 {
        return Err("dport-map-readback");
    }
    reg_write(MAP_REG, 5);
    if reg_read(MAP_REG) != 5 {
        return Err("dport-map-rewrite");
    }
    Ok(())
}

// ── dma: not modeled on ESP32-classic ─────────────────────────────────────
//
// The ESP32-classic has a non-GDMA DMA controller (AHB DMA, "GPDMA" in
// esp-idf parlance). No model exists for this peripheral in the simulator
// today (`crates/core/src/peripherals/esp32/` has no dma.rs and
// `configs/chips/esp32.yaml` lists no dma/gdma peripheral). We report FAIL
// with an honest model-gap code rather than silently skipping.
fn check_dma() -> Result<(), &'static str> {
    Err("esp32-no-dma-model")
}

#[esp_hal::main]
fn main() -> ! {
    let _peripherals = esp_hal::init(esp_hal::Config::default());

    report("clock", check_clock());
    report("gpio", check_gpio());
    report("timer", check_timer());
    report("irq", check_irq());
    report("dma", check_dma());
    uart0_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
