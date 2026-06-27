//! ESP32-C3 Tier-1 fixture firmware.
//!
//! Validates the simulator's ESP32-C3 chip model peripheral-by-peripheral with
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
//! Classes whose peripheral the chip YAML does not declare are omitted; the
//! harness maps them to `na` via the YAML peripheral-id heuristic.
//!
//! # Peripheral coverage against esp32c3.yaml
//!
//! | YAML id           | CLASS_MARKER  | class  | status  |
//! |-------------------|---------------|--------|---------|
//! | uart0             | uart          | uart   | implicit PASS via `done` |
//! | gpio              | gpio          | gpio   | PASS — OUT round-trip    |
//! | timg0             | timg          | timer  | FAIL — declarative model has no counter advance |
//! | interrupt_core0   | interrupt     | irq    | PASS — MAP register round-trip |
//! | (no systimer)     | —             | clock  | na      |
//! | (no gdma/dma)     | —             | dma    | na      |
//!
//! Register offsets and bit positions follow the ESP32-C3 TRM and are
//! cross-checked against the simulator's declarative models in
//! `configs/peripherals/esp32c3/`.

#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

// ── Peripheral base addresses (ESP32-C3 TRM §3.3 memory map) ──────────────
const UART0_BASE: u32 = 0x6000_0000;
const GPIO_BASE: u32 = 0x6000_4000;
const TIMG0_BASE: u32 = 0x6001_F000;
const INTMATRIX_BASE: u32 = 0x600C_2000;
const I2C0_BASE: u32 = 0x6001_3000;

#[inline(always)]
fn reg_read(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
fn reg_write(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

// ── UART0 raw byte output ─────────────────────────────────────────────────
//
// The simulator's UART model (crates/core/src/peripherals/uart.rs) is
// instantiated with the default STM32F1 register layout, which maps:
//   offset 0x00 → legacy TX alias (writes push a byte to stdout)
//   offset 0x04 → DR (normal TX/RX)
//
// ESP32-C3 UART0 FIFO is at offset 0x00 (TRM §26). Both the TRM address and
// the simulator's STM32F1 alias resolve to the same physical write: a byte
// pushed straight to the simulator's TX sink. No STATUS poll is needed because
// the simulator model never reports TX-FIFO-full.
fn uart0_write_byte(byte: u8) {
    // Byte-write to FIFO: the simulator dispatches u8 writes at offset 0x00
    // through the STM32F1 `is_legacy_tx_alias` path in uart.rs:362.
    unsafe {
        core::ptr::write_volatile(UART0_BASE as *mut u8, byte);
    }
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

// ── gpio: write OUT directly; read back ───────────────────────────────────
//
// configs/peripherals/esp32c3/gpio.yaml (declarative, timing: null):
//   OUT @ offset 0x04 (READ_WRITE, reset 0)
//
// The declarative model stores plain writes to OUT. W1TS/W1TC (offsets 0x08,
// 0x0C) are independent storage registers without set-clear side effects in
// the declarative engine (side_effects: null). OUT is therefore the canonical
// test target: write a value, read back the same register, compare.
fn check_gpio() -> Result<(), &'static str> {
    const OUT: u32 = GPIO_BASE + 0x04;
    const PAT: u32 = 1 << 4; // GPIO4 — no boot-strap function on C3

    reg_write(OUT, PAT);
    if reg_read(OUT) & PAT == 0 {
        return Err("gpio-out-store");
    }
    reg_write(OUT, 0);
    if reg_read(OUT) & PAT != 0 {
        return Err("gpio-out-clear");
    }
    Ok(())
}

// ── timer: timg0 counter advance check ────────────────────────────────────
//
// configs/peripherals/esp32c3/timg0.yaml (declarative, timing: null):
//   T0CONFIG @ 0x00 (EN=bit31, INCREASE=bit30, DIVIDER=bits[28:13])
//   T0LO     @ 0x04 (counter snapshot low word)
//   T0HI     @ 0x08 (counter snapshot high word)
//   T0UPDATE @ 0x0C (write any value → latch live counter into T0LO/HI)
//
// The declarative model has no periodic timing hooks for the counter
// (timing: null). Writing T0CONFIG with EN|INCREASE does not start an
// auto-incrementing counter; T0LO/T0HI remain at their reset value (0) after
// T0UPDATE writes. This is an honest model gap — the timer model is
// register-file storage only, not a live counter.
fn check_timer() -> Result<(), &'static str> {
    const T0CONFIG: u32 = TIMG0_BASE + 0x00;
    const T0LO: u32 = TIMG0_BASE + 0x04;
    const T0HI: u32 = TIMG0_BASE + 0x08;
    const T0UPDATE: u32 = TIMG0_BASE + 0x0C;
    const EN: u32 = 1 << 31;
    const INCREASE: u32 = 1 << 30;
    const DIVIDER_1: u32 = 1 << 13;

    // Verify T0CONFIG is writable (basic register store).
    reg_write(T0CONFIG, EN | INCREASE | DIVIDER_1);
    if reg_read(T0CONFIG) & EN == 0 {
        return Err("timg0-config-store");
    }

    // Latch the counter twice with a spin gap; expect the value to advance.
    let latch = || -> u64 {
        reg_write(T0UPDATE, 1);
        ((reg_read(T0HI) as u64) << 32) | reg_read(T0LO) as u64
    };

    let a = latch();
    // Bounded deterministic spin — ~20 000 iterations in the simulator.
    for i in 0u32..20_000 {
        core::hint::black_box(i);
    }
    let b = latch();

    if b > a {
        Ok(())
    } else {
        // Declarative model has no live counter: T0LO/T0HI stay at 0.
        Err("timg0-not-counting")
    }
}

// ── irq: interrupt_core0 MAP register write-then-readback ─────────────────
//
// configs/peripherals/esp32c3/interrupt_core0.yaml (declarative):
//   Registers at offsets 0, 4, 8, … 4*N for source N.
//   I2C_EXT0_INTR_MAP @ offset 116 → source index 29 (116 / 4 = 29).
//   Each MAP register is READ_WRITE, bits [4:0] hold the CPU interrupt slot.
//
// The check writes a slot value, reads it back, then writes a second value to
// confirm overwrite. No interrupt is actually delivered (INT_ENA = 0 throughout).
fn check_irq() -> Result<(), &'static str> {
    // I2C_EXT0_INTR_MAP is at address_offset 116 in the interrupt_core0
    // peripheral (configs/peripherals/esp32c3/interrupt_core0.yaml, line 353).
    // Source index = 116 / 4 = 29.
    const I2C_EXT0_SOURCE: u32 = 29;
    const MAP_REG: u32 = INTMATRIX_BASE + 4 * I2C_EXT0_SOURCE;

    reg_write(MAP_REG, 17);
    if reg_read(MAP_REG) != 17 {
        return Err("intmatrix-map-readback");
    }
    reg_write(MAP_REG, 5);
    if reg_read(MAP_REG) != 5 {
        return Err("intmatrix-map-rewrite");
    }
    Ok(())
}

// ── i2c: run a command list through the I2C0 transaction engine ───────────
//
// configs/chips/esp32c3.yaml wires i2c0 (type esp32c3_i2c) — a behavioral
// model with the COMD command-list engine
// (crates/core/src/peripherals/esp32c3/i2c.rs), not the declarative descriptor.
// Program RSTART→WRITE(1)→STOP, push an address byte into the TX FIFO, then set
// CTR.TRANS_START. With no slave wired the WRITE is NACKed, but the engine still
// walks the list to STOP: every executed slot latches COMD.command_done (bit 31)
// and the run raises INT_RAW.TRANS_COMPLETE (bit 7). That state transition is
// the proof the transaction engine ran (a declarative register file could not
// produce it).
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

    reg_write(INT_CLR, 0xFFFF_FFFF); // clear any stale raw-int state
    reg_write(FIFO_CONF, (1 << 12) | (1 << 13)); // RX/TX FIFO reset (self-clearing)
    reg_write(CMD0, cmd(6, 0)); // RSTART
    reg_write(CMD0 + 4, cmd(1, 1)); // WRITE 1 byte (the address)
    reg_write(CMD0 + 8, cmd(2, 0)); // STOP
    reg_write(DATA, 0xA0); // address byte into TX FIFO

    if reg_read(INT_RAW) & TRANS_COMPLETE != 0 {
        return Err("i2c-complete-early"); // must not be set before TRANS_START
    }
    reg_write(CTR, TRANS_START);

    if reg_read(INT_RAW) & TRANS_COMPLETE == 0 {
        return Err("i2c-no-complete");
    }
    if reg_read(CMD0) & CMD_DONE == 0 {
        return Err("i2c-cmd-not-done");
    }
    // TRANS_START is self-clearing once the list has run.
    if reg_read(CTR) & TRANS_START != 0 {
        return Err("i2c-start-stuck");
    }
    Ok(())
}

#[entry]
fn main() -> ! {
    // gpio, timer, irq, i2c — ordered by increasing complexity.
    // clock/dma: esp32c3.yaml wires system/rtc_cntl/dma as declarative register
    // files only (no behavioral engine) → left unrecorded, not faked.
    report("gpio", check_gpio());
    report("timer", check_timer());
    report("irq", check_irq());
    report("i2c", check_i2c());
    uart0_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
