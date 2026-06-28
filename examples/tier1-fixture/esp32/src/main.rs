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
// SPI3 / VSPI controller (TRM §7). Free general-purpose SPI on classic ESP32
// (SPI0/SPI1 are the flash controllers). Wired as a real Esp32Spi model.
const SPI3_BASE: u32 = 0x3FF6_5000;
// I2C0 / I2C_EXT0 controller (TRM §11). Real command-list engine; a BMP280 is
// attached on the bus at address 0x76 by configure_xtensa_esp32.
const I2C0_BASE: u32 = 0x3FF5_3000;
// SENS block (TRM §29.4) — the classic ESP32 has no APB_SARADC; the one-shot
// ("RTC controller") ADC path the IDF adc1_get_raw driver uses lives here.
// Wired as a real Esp32SarAdc model by configure_xtensa_esp32.
const SENS_BASE: u32 = 0x3FF4_8800;

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

// ── adc: SENS SAR-ADC one-shot, channel- and width-dependent result ────────
//
// TRM §29.4 (SENS block at 0x3FF4_8800 — the classic ESP32's one-shot ADC path,
// distinct from the C3/S3 APB_SARADC):
//   SAR_READ_CTRL   @ 0x00 — bits[17:16] SAR1_SAMPLE_BIT (00=9-bit … 11=12-bit)
//   SAR_MEAS_START1 @ 0x54 — bits[30:19] SAR1_EN_PAD (one-hot channel bitmap),
//                            bit 18 MEAS1_START_FORCE, bit 17 MEAS1_START_SAR
//                            (trigger), bit 16 MEAS1_DONE_SAR (RO), bits[15:0]
//                            MEAS1_DATA_SAR (RO, the sample).
//
// The model (`crates/core/src/peripherals/esp32/sar_adc.rs`) decodes the
// one-hot channel, produces a deterministic channel-dependent 12-bit code
// scaled to the configured resolution, latches it into DATA and raises DONE.
// A round-trip register stub cannot (a) raise the RO DONE bit, (b) return a
// channel-dependent DATA, or (c) scale DATA with the configured width — so each
// assert below proves genuine conversion behaviour, not a register echo.
fn check_adc() -> Result<(), &'static str> {
    const READ_CTRL: u32 = SENS_BASE + 0x00;
    const MEAS_START1: u32 = SENS_BASE + 0x54;
    const START_FORCE: u32 = 1 << 18;
    const START_SAR: u32 = 1 << 17;
    const DONE_SAR: u32 = 1 << 16;
    const DATA_MASK: u32 = 0xFFFF;
    const EN_PAD_SHIFT: u32 = 19;
    const SAMPLE_BIT_SHIFT: u32 = 16;

    // Set SAR1 resolution, then trigger a one-shot of `channel` and return its
    // DATA field once DONE latches (bounded poll — the conversion is synchronous
    // in the model, so DONE is set on the START write).
    let read_channel = |channel: u32, sample_bit: u32| -> Result<u32, &'static str> {
        reg_write(READ_CTRL, sample_bit << SAMPLE_BIT_SHIFT);
        let en_pad = 1u32 << channel;
        reg_write(MEAS_START1, (en_pad << EN_PAD_SHIFT) | START_FORCE | START_SAR);
        for _ in 0..10_000 {
            if reg_read(MEAS_START1) & DONE_SAR != 0 {
                return Ok(reg_read(MEAS_START1) & DATA_MASK);
            }
        }
        Err("adc-done-never-set")
    };

    // 12-bit conversions on two distinct channels must yield distinct, nonzero
    // results — proof the sample tracks the selected channel.
    let d3 = read_channel(3, 3)?;
    if d3 == 0 {
        return Err("adc-ch3-zero");
    }
    let d5 = read_channel(5, 3)?;
    if d5 == 0 {
        return Err("adc-ch5-zero");
    }
    if d3 == d5 {
        return Err("adc-channel-independent");
    }

    // Same channel at 9-bit resolution must equal the 12-bit value >> 3 — proof
    // the result scales with the configured width (a real lower-res SAR).
    let d5_9bit = read_channel(5, 0)?;
    if d5_9bit != d5 >> 3 {
        return Err("adc-width-not-scaled");
    }
    Ok(())
}

// ── dma: not a general-purpose mem→mem controller on ESP32-classic ─────────
//
// Honest model gap. Unlike the C3/S3 (which have a central GDMA capable of
// mem→mem copies), the classic ESP32 has NO general-purpose DMA engine: DMA is
// per-peripheral linked-list (SPI/I2S/SDMMC/AES/SHA), each moving data between
// memory and that peripheral's data path — never memory→memory. There is no
// transfer a fixture could fire and prove by checking a destination buffer
// received source bytes + an EOF flag without also modelling a full peripheral
// pipeline. So no `dma`/`gdma` peripheral is declared in `configs/chips/esp32.yaml`
// (the matrix renders the cell `na`) and we report an honest gap code here.
fn check_dma() -> Result<(), &'static str> {
    Err("esp32-no-mem2mem-dma")
}

// ── spi: FIFO round-trip + CMD.USR synchronous self-clear on SPI3 ──────────
//
// TRM §7 (SPI Controller, ESP32-classic offsets):
//   SPI_CMD_REG       @ 0x00 — bit 18 (USR) = start; cleared on completion.
//   SPI_USER_REG      @ 0x1C — bit 27 (USR_MOSI) requests the MOSI phase.
//   SPI_MOSI_DLEN_REG @ 0x28 — MOSI bit length minus 1.
//   SPI_W0..W15       @ 0x80 — 64-byte data FIFO.
//
// The model (`crates/core/src/peripherals/esp32/spi.rs`) backs the FIFO with
// real storage and, on a CMD write with USR set, synchronously streams the
// MOSI bytes and clears CMD.USR. A read-as-zero / round-trip stub would leave
// USR set (the busy-poll would hang). So both the FIFO round-trip and the
// self-clearing busy bit are behaviour a stub cannot fake. No SPI device is
// attached in the tier-1 setup; the streamed bytes go nowhere, which is fine —
// we are validating the controller, not a peripheral on the bus.
fn check_spi() -> Result<(), &'static str> {
    const CMD: u32 = SPI3_BASE + 0x00;
    const USER: u32 = SPI3_BASE + 0x1C;
    const MOSI_DLEN: u32 = SPI3_BASE + 0x28;
    const W0: u32 = SPI3_BASE + 0x80;
    const CMD_USR: u32 = 1 << 18;
    const USER_USR_MOSI: u32 = 1 << 27;

    // FIFO round-trip: write two data words and read them back verbatim.
    reg_write(W0, 0xDEAD_BEEF);
    reg_write(W0 + 4, 0x0102_0304);
    if reg_read(W0) != 0xDEAD_BEEF {
        return Err("spi-fifo-w0");
    }
    if reg_read(W0 + 4) != 0x0102_0304 {
        return Err("spi-fifo-w1");
    }

    // Behavioural: arm a 4-byte MOSI transfer, fire CMD.USR, and confirm the
    // controller cleared the USR bit synchronously (transaction completed).
    reg_write(USER, USER_USR_MOSI);
    reg_write(MOSI_DLEN, (4 * 8) - 1);
    reg_write(CMD, CMD_USR);
    if reg_read(CMD) & CMD_USR != 0 {
        return Err("spi-cmd-usr-not-cleared");
    }
    Ok(())
}

// ── i2c: command-list engine drives a real BMP280 register-pointer read ────
//
// TRM §11 (I2C controller, ESP32-classic offsets):
//   I2C_CTR_REG       @ 0x04 — bit 5 (TRANS_START) = fire; self-clears.
//   I2C_SR_REG        @ 0x08 — bit 0 = ACK_REC (slave acked).
//   I2C_FIFO_CONF_REG @ 0x18 — bit 13 = TX_FIFO_RST, bit 12 = RX_FIFO_RST.
//   I2C_DATA_REG      @ 0x1C — write→TX FIFO, read→pop RX FIFO.
//   I2C_INT_RAW_REG   @ 0x20 — bit 7 = TRANS_COMPLETE, bit 10 = NACK(ACK_ERR).
//   I2C_INT_CLR_REG   @ 0x24 — write 1 to clear matching INT_RAW bits.
//   I2C_COMD0..15_REG @ 0x58 — 16 command slots.
//
// Classic-ESP32 command opcodes (hal/esp32/include/hal/i2c_ll.h):
//   RSTART=0, WRITE=1, READ=2, STOP=3, END=4.  (C3/S3 renumber these.)
//
// The model (`crates/core/src/peripherals/esp32/i2c.rs`) runs the COMD list on
// TRANS_START: it pops the addr byte, matches the attached BMP280 at 0x76,
// delivers the register pointer, then a repeated-start READ pulls the device's
// CHIP_ID (0x58) into the RX FIFO and STOP raises TRANS_COMPLETE. A round-trip
// stub could never (a) clear TRANS_START, (b) raise TRANS_COMPLETE, (c) raise
// NACK for an absent address, or (d) return device-specific data — so each of
// these asserts genuine engine behaviour.
fn check_i2c() -> Result<(), &'static str> {
    const CTR: u32 = I2C0_BASE + 0x04;
    const SR: u32 = I2C0_BASE + 0x08;
    const FIFO_CONF: u32 = I2C0_BASE + 0x18;
    const DATA: u32 = I2C0_BASE + 0x1C;
    const INT_RAW: u32 = I2C0_BASE + 0x20;
    const INT_CLR: u32 = I2C0_BASE + 0x24;
    const COMD0: u32 = I2C0_BASE + 0x58;

    const TRANS_START: u32 = 1 << 5;
    const ACK_REC: u32 = 1 << 0;
    const TX_FIFO_RST: u32 = 1 << 13;
    const RX_FIFO_RST: u32 = 1 << 12;
    const TRANS_COMPLETE: u32 = 1 << 7;
    const NACK: u32 = 1 << 10;

    // Opcode encode: bits[13:11] = op, bits[7:0] = byte_num.
    const fn cmd(op: u32, n: u32) -> u32 {
        ((op & 0x7) << 11) | (n & 0xFF)
    }
    const OP_RSTART: u32 = 0;
    const OP_WRITE: u32 = 1;
    const OP_READ: u32 = 2;
    const OP_STOP: u32 = 3;

    let clear_ints = || reg_write(INT_CLR, 0xFFFF_FFFF);

    // ── Phase 1: addressing an ABSENT device (0x20) must raise NACK ──────────
    reg_write(FIFO_CONF, TX_FIFO_RST | RX_FIFO_RST);
    clear_ints();
    reg_write(COMD0, cmd(OP_RSTART, 0));
    reg_write(COMD0 + 4, cmd(OP_WRITE, 1));
    reg_write(COMD0 + 8, cmd(OP_STOP, 0));
    reg_write(DATA, 0x20 << 1); // addr 0x20 + W; nothing attached there
    reg_write(CTR, TRANS_START);
    if reg_read(CTR) & TRANS_START != 0 {
        return Err("i2c-trans-start-stuck");
    }
    if reg_read(INT_RAW) & NACK == 0 {
        return Err("i2c-absent-no-nack");
    }

    // ── Phase 2: real read of the BMP280 CHIP_ID (0x58) at register 0xD0 ─────
    reg_write(FIFO_CONF, TX_FIFO_RST | RX_FIFO_RST);
    clear_ints();
    // RSTART; WRITE 2 (addr+W, ptr=0xD0); RSTART; WRITE 1 (addr+R); READ 1; STOP
    reg_write(COMD0, cmd(OP_RSTART, 0));
    reg_write(COMD0 + 4, cmd(OP_WRITE, 2));
    reg_write(COMD0 + 8, cmd(OP_RSTART, 0));
    reg_write(COMD0 + 12, cmd(OP_WRITE, 1));
    reg_write(COMD0 + 16, cmd(OP_READ, 1));
    reg_write(COMD0 + 20, cmd(OP_STOP, 0));
    reg_write(DATA, 0x76 << 1); // addr+W (0xEC)
    reg_write(DATA, 0xD0); // register pointer = CHIP_ID
    reg_write(DATA, (0x76 << 1) | 1); // addr+R (0xED)
    reg_write(CTR, TRANS_START);

    if reg_read(INT_RAW) & NACK != 0 {
        return Err("i2c-bmp280-nacked");
    }
    if reg_read(SR) & ACK_REC == 0 {
        return Err("i2c-no-ack-rec");
    }
    if reg_read(INT_RAW) & TRANS_COMPLETE == 0 {
        return Err("i2c-no-trans-complete");
    }
    if reg_read(DATA) != 0x58 {
        return Err("i2c-bad-chip-id");
    }
    Ok(())
}

#[esp_hal::main]
fn main() -> ! {
    let _peripherals = esp_hal::init(esp_hal::Config::default());

    report("clock", check_clock());
    report("gpio", check_gpio());
    report("timer", check_timer());
    report("irq", check_irq());
    report("adc", check_adc());
    report("dma", check_dma());
    report("spi", check_spi());
    report("i2c", check_i2c());
    uart0_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
