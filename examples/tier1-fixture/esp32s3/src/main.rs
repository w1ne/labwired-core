//! ESP32-S3 Tier-1 fixture firmware.
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
//! positions follow the ESP32-S3 TRM and are cross-checked against the
//! ESP-IDF `soc/esp32s3/register` headers; identical layouts are documented
//! in the simulator's model sources (`crates/core/src/peripherals/esp32s3/`).

#![no_std]
#![no_main]

use esp_backtrace as _;

// ESP-IDF app descriptor — required by `espflash save-image` so the 2nd-stage
// bootloader accepts the app image on the faithful (`--rom-boot`) path.
esp_bootloader_esp_idf::esp_app_desc!();

// ── Peripheral base addresses (ESP32-S3 TRM §3.3 memory map) ──────────────
const UART0_BASE: u32 = 0x6000_0000;
const SYSTIMER_BASE: u32 = 0x6002_3000;
const GPIO_BASE: u32 = 0x6000_4000;
const TIMG0_BASE: u32 = 0x6001_F000;
const INTMATRIX_BASE: u32 = 0x600C_2000; // DR_REG_INTERRUPT_BASE (CORE0 map)
const GDMA_BASE: u32 = 0x6003_F000;
const MCPWM0_BASE: u32 = 0x6001_E000;
const RMT_BASE: u32 = 0x6001_6000;
const I2C0_BASE: u32 = 0x6001_3000;

#[inline(always)]
fn reg_read(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
fn reg_write(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

/// Fixed-iteration busy spin. Deterministic in the simulator (1 iteration is
/// a handful of CPU cycles); `black_box` keeps the loop from being folded.
fn spin(iters: u32) {
    for i in 0..iters {
        core::hint::black_box(i);
    }
}

// ── UART0 protocol output (raw register writes) ───────────────────────────
//
// TRM §26 (`uart_reg.h`, UART0 @ 0x6000_0000): FIFO @ 0x00 (TX push),
// STATUS @ 0x1C (TXFIFO_CNT in bits [25:16]). The protocol lines are pushed
// straight into the UART0 TX FIFO. Deliberately NOT the ROM's
// `uart_tx_one_char` (esp-println's `uart` backend): the real S3 ROM console
// multiplexes every character to both UART0 *and* USB-Serial-JTAG, and the
// two host echo streams interleave mid-line, garbling the protocol. A single
// FIFO serializes everything.
fn uart0_write_byte(byte: u8) {
    const FIFO: u32 = UART0_BASE + 0x00;
    const STATUS: u32 = UART0_BASE + 0x1C;
    const TXFIFO_LEN: u32 = 128; // SOC_UART_FIFO_LEN

    // Bounded poll for FIFO space (deterministic sim — fixed iterations).
    for _ in 0..1_000_000 {
        if ((reg_read(STATUS) >> 16) & 0x3FF) < TXFIFO_LEN {
            break;
        }
    }
    // On timeout, write anyway; a garbled line beats a hung fixture.
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

// ── clock: SYSTIMER unit0 advances between two latched reads ──────────────
//
// TRM §16.5: UNIT0_OP @ 0x04 (write bit 30 = TIMER_UNIT0_UPDATE to latch a
// snapshot; bit 29 = VALUE_VALID), UNIT0_VALUE_HI/LO @ 0x40/0x44. UNIT0 runs
// at reset (CONF @ 0x00 reset 0x4600_0000, bit 30 UNIT0_WORK_EN set) and is
// clocked at 16 MHz independent of the CPU clock.
fn check_clock() -> Result<(), &'static str> {
    const UNIT0_OP: u32 = SYSTIMER_BASE + 0x04;
    const UNIT0_VALUE_HI: u32 = SYSTIMER_BASE + 0x40;
    const UNIT0_VALUE_LO: u32 = SYSTIMER_BASE + 0x44;
    const OP_UPDATE: u32 = 1 << 30;
    const OP_VALUE_VALID: u32 = 1 << 29;

    let latch = || -> Result<u64, &'static str> {
        reg_write(UNIT0_OP, OP_UPDATE);
        // Bounded VALUE_VALID poll (the model asserts it immediately; real
        // silicon needs a few cycles).
        let mut valid = false;
        for _ in 0..10_000 {
            if reg_read(UNIT0_OP) & OP_VALUE_VALID != 0 {
                valid = true;
                break;
            }
        }
        if !valid {
            return Err("systimer-value-valid-timeout");
        }
        Ok(((reg_read(UNIT0_VALUE_HI) as u64) << 32) | reg_read(UNIT0_VALUE_LO) as u64)
    };

    let t1 = latch()?;
    spin(20_000);
    let t2 = latch()?;
    if t2 > t1 {
        Ok(())
    } else {
        Err("systimer-not-advancing")
    }
}

// ── gpio: ENABLE_W1TS + OUT_W1TS/W1TC on GPIO4, read back via OUT ─────────
//
// TRM §5.5: OUT @ 0x04, OUT_W1TS @ 0x08, OUT_W1TC @ 0x0C, ENABLE @ 0x20,
// ENABLE_W1TS @ 0x24, ENABLE_W1TC @ 0x28. GPIO4 carries no boot strap.
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

// ── timer: TIMG0 T0 enabled + increase; two T0UPDATE latches differ ───────
//
// TRM §13.4: T0CONFIG @ 0x00 (bit 31 EN, bit 30 INCREASE, bits[28:13]
// DIVIDER), T0LO @ 0x04, T0HI @ 0x08, T0UPDATE @ 0x0C (write any value to
// snapshot the live counter into T0LO/HI).
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

// ── irq: intmatrix mapping register write + read-back ─────────────────────
//
// TRM §9.4: INTERRUPT_CORE0_<source>_MAP_REG @ INTMATRIX_BASE + 4*source_id;
// the register holds the CPU interrupt slot (bits [4:0]). Use a real source
// row: I2C_EXT0 = ETS_I2C_EXT0_INTR_SOURCE = 42. This proves the matrix
// wiring (slot binding round-trips); delivery is covered by
// `crates/core/tests/intmatrix_alarm.rs` (SYSTIMER alarm → intmatrix → CPU
// vector). The binding stays inert: every check here polls INT_RAW with
// INT_ENA = 0, so no source ever asserts.
fn check_irq() -> Result<(), &'static str> {
    const I2C_EXT0_SOURCE: u32 = 42;
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

// ── dma: honest GDMA memory-to-memory attempt ──────────────────────────────
//
// gdma_reg.h: per-channel stride 0xC0; ch0 IN_CONF0 @ 0x00 (bit 4
// MEM_TRANS_EN selects memory-to-memory), IN_INT_RAW @ 0x08 (bit 1
// IN_SUC_EOF), IN_INT_CLR @ 0x14, IN_LINK @ 0x20 (addr[19:0], START bit 22),
// OUT_INT_CLR @ 0x74, OUT_LINK @ 0x80 (addr[19:0], START bit 21).
//
// The check builds real linked-list descriptors over real DRAM buffers,
// kicks OUT then IN, waits for IN_SUC_EOF and then verifies the bytes
// actually moved. The current model latches EOF without walking the
// descriptor list (documented limitation in the model source), so this
// reports FAIL code=gdma-no-m2m-model until the model grows real m2m moves.
#[repr(C, align(4))]
struct DmaDescriptor {
    /// owner(31) | suc_eof(30) | length[23:12] | size[11:0]
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
    const IN_CONF0: u32 = GDMA_BASE + 0x00;
    const IN_INT_RAW: u32 = GDMA_BASE + 0x08;
    const IN_INT_CLR: u32 = GDMA_BASE + 0x14;
    const IN_LINK: u32 = GDMA_BASE + 0x20;
    const OUT_INT_CLR: u32 = GDMA_BASE + 0x74;
    const OUT_LINK: u32 = GDMA_BASE + 0x80;
    const MEM_TRANS_EN: u32 = 1 << 4;
    const IN_SUC_EOF: u32 = 1 << 1;
    const INLINK_START: u32 = 1 << 22;
    const OUTLINK_START: u32 = 1 << 21;
    const LINK_ADDR_MASK: u32 = 0x000F_FFFF;

    // Minimal enable round-trip: MEM_TRANS_EN must store and read back.
    reg_write(IN_CONF0, MEM_TRANS_EN);
    if reg_read(IN_CONF0) & MEM_TRANS_EN == 0 {
        return Err("gdma-conf-roundtrip");
    }

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

    reg_write(IN_INT_CLR, 0xFFFF_FFFF);
    reg_write(OUT_INT_CLR, 0xFFFF_FFFF);
    reg_write(IN_LINK, (rx_desc_addr & LINK_ADDR_MASK) | INLINK_START);
    reg_write(OUT_LINK, (tx_desc_addr & LINK_ADDR_MASK) | OUTLINK_START);

    let mut eof = false;
    for _ in 0..100_000 {
        if reg_read(IN_INT_RAW) & IN_SUC_EOF != 0 {
            eof = true;
            break;
        }
    }
    if !eof {
        return Err("gdma-eof-timeout");
    }
    // EOF alone is not a transfer: the bytes must actually have moved. Both
    // buffers are read VOLATILE through the raw pointers — a DMA write is
    // invisible to the compiler, so a plain read of DMA_DST could be folded
    // to its all-zero initializer and freeze this check at
    // `gdma-no-m2m-model` even after the model gains real m2m moves.
    let moved = (0..len).all(|i| unsafe {
        core::ptr::read_volatile((src_addr + i) as *const u8)
            == core::ptr::read_volatile((dst_addr + i) as *const u8)
    });
    if moved {
        Ok(())
    } else {
        Err("gdma-no-m2m-model")
    }
}

// ── mcpwm: timer0 prescale/period/continuous-up, poll INT_RAW for TEZ ─────
//
// mcpwm_reg.h (MCPWM0 @ 0x6001_E000): CLK_CFG @ 0x000 (PWM_clk =
// MCPWM_clk/(PRESCALE+1)), TIMER0_CFG0 @ 0x004 (PRESCALE[7:0],
// PERIOD[23:8]), TIMER0_CFG1 @ 0x008 (MODE[2:0]: 1 = count up; START[4:3]:
// 2 = run continuously), INT_RAW @ 0x114 (bit 0 = TIMER0_TEZ), INT_CLR @
// 0x11C (W1C). TEZ latches when the up-counter wraps from PERIOD to 0.
fn check_mcpwm() -> Result<(), &'static str> {
    const CLK_CFG: u32 = MCPWM0_BASE + 0x000;
    const TIMER0_CFG0: u32 = MCPWM0_BASE + 0x004;
    const TIMER0_CFG1: u32 = MCPWM0_BASE + 0x008;
    const INT_RAW: u32 = MCPWM0_BASE + 0x114;
    const INT_CLR: u32 = MCPWM0_BASE + 0x11C;
    const TEZ0: u32 = 1 << 0;
    const MODE_UP: u32 = 1; // CFG1[2:0]
    const START_CONTINUOUS: u32 = 2 << 3; // CFG1[4:3]

    reg_write(CLK_CFG, 0); // PWM_clk = MCPWM_clk / 1
    reg_write(TIMER0_CFG0, 50 << 8); // PRESCALE=0, PERIOD=50
    reg_write(INT_CLR, 0x3FFF_FFFF);
    reg_write(TIMER0_CFG1, MODE_UP | START_CONTINUOUS);

    for _ in 0..100_000 {
        if reg_read(INT_RAW) & TEZ0 != 0 {
            reg_write(TIMER0_CFG1, 0); // freeze the timer again
            reg_write(INT_CLR, 0x3FFF_FFFF);
            return Ok(());
        }
    }
    Err("mcpwm-tez-timeout")
}

// ── rmt: end-marker into ch0 RAM, tx_start, poll INT_RAW tx_end ────────────
//
// rmt_reg.h (RMT @ 0x6001_6000): CH0DATA @ 0x00 (APB FIFO into channel RAM),
// CH0CONF0 @ 0x20 (bit 0 TX_START — self-clearing write-trigger; bit 24
// CONF_UPDATE), INT_RAW @ 0x70 (bit 0 = CH0_TX_END), INT_CLR @ 0x7C (W1C).
fn check_rmt() -> Result<(), &'static str> {
    const CH0DATA: u32 = RMT_BASE + 0x00;
    const CH0CONF0: u32 = RMT_BASE + 0x20;
    const INT_RAW: u32 = RMT_BASE + 0x70;
    const INT_CLR: u32 = RMT_BASE + 0x7C;
    const TX_START: u32 = 1 << 0;
    const CONF_UPDATE: u32 = 1 << 24;
    const CH0_TX_END: u32 = 1 << 0;

    reg_write(INT_CLR, 0x3FFF_FFFF);
    // One RMT entry: both durations 0 = TX end marker.
    reg_write(CH0DATA, 0);
    // Preserve the channel config (divider/mem_size/carrier), strobe
    // CONF_UPDATE + TX_START.
    let conf0 = reg_read(CH0CONF0);
    reg_write(CH0CONF0, conf0 | CONF_UPDATE | TX_START);

    for _ in 0..100_000 {
        if reg_read(INT_RAW) & CH0_TX_END != 0 {
            reg_write(INT_CLR, CH0_TX_END);
            return Ok(());
        }
    }
    Err("rmt-tx-end-timeout")
}

// ── i2c: RSTART/WRITE/STOP command list, trans_start, poll trans_complete ──
//
// TRM §29 (I2C0 @ 0x6001_3000): CTR @ 0x04 (bit 5 TRANS_START,
// self-clearing), FIFO_CONF @ 0x18 (bit 12 RX_FIFO_RST, bit 13 TX_FIFO_RST,
// self-clearing), DATA @ 0x1C (TX FIFO push), INT_RAW @ 0x20 (bit 7
// TRANS_COMPLETE, bit 10 NACK), INT_CLR @ 0x24 (W1C), COMD0..7 @ 0x58..0x74
// (OPCODE[13:11]: 1=WRITE 2=STOP 3=READ 4=END 6=RSTART; BYTE_NUM[7:0]).
//
// Addresses TMP102 @ 0x48 (pointer-register write). A NACK is acceptable —
// the check proves the command engine completes; a dead engine is not.
fn check_i2c() -> Result<(), &'static str> {
    const CTR: u32 = I2C0_BASE + 0x04;
    const FIFO_CONF: u32 = I2C0_BASE + 0x18;
    const DATA: u32 = I2C0_BASE + 0x1C;
    const INT_RAW: u32 = I2C0_BASE + 0x20;
    const INT_CLR: u32 = I2C0_BASE + 0x24;
    const COMD0: u32 = I2C0_BASE + 0x58;
    const COMD1: u32 = I2C0_BASE + 0x5C;
    const COMD2: u32 = I2C0_BASE + 0x60;
    const TRANS_START: u32 = 1 << 5;
    const TRANS_COMPLETE: u32 = 1 << 7;
    const FIFO_CONF_RESET: u32 = 0x0000_408B; // SVD reset (watermark thresholds)
    const CTR_RESET: u32 = 0x0000_020B; // SVD reset (SCL/SDA force-out etc.)

    const OP_WRITE: u32 = 1 << 11;
    const OP_STOP: u32 = 2 << 11;
    const OP_RSTART: u32 = 6 << 11;

    // Flush both FIFOs (self-clearing reset pulse bits) and clear stale ints.
    reg_write(FIFO_CONF, FIFO_CONF_RESET | (1 << 12) | (1 << 13));
    reg_write(INT_CLR, 0x3FFF_FFFF);

    // TX FIFO: address byte (0x48 << 1 | W) + one data byte (pointer reg 0).
    reg_write(DATA, (0x48 << 1) | 0);
    reg_write(DATA, 0x00);

    // Command list: RSTART, WRITE 2 bytes, STOP.
    reg_write(COMD0, OP_RSTART);
    reg_write(COMD1, OP_WRITE | 2);
    reg_write(COMD2, OP_STOP);

    reg_write(CTR, CTR_RESET | TRANS_START);

    for _ in 0..100_000 {
        if reg_read(INT_RAW) & TRANS_COMPLETE != 0 {
            reg_write(INT_CLR, 0x3FFF_FFFF);
            return Ok(());
        }
    }
    Err("i2c-trans-complete-timeout")
}

#[esp_hal::main]
fn main() -> ! {
    let _peripherals = esp_hal::init(esp_hal::Config::default());

    report("clock", check_clock());
    report("gpio", check_gpio());
    report("timer", check_timer());
    report("irq", check_irq());
    report("dma", check_dma());
    report("mcpwm", check_mcpwm());
    report("rmt", check_rmt());
    report("i2c", check_i2c());
    uart0_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
