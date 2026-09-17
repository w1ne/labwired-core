// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GDMA (General DMA) controller for ESP32-S3.
//!
//! Base = `DR_REG_GDMA_BASE` = `0x6003_F000`. The GDMA has **5 channels**,
//! each carrying an independent **IN (RX)** datapath and **OUT (TX)**
//! datapath, plus a global `MISC_CONF`. Peripherals (SPI2/3, I2S, ADC,
//! AES, SHA, …) are bound to a channel and the channel walks an in-RAM
//! linked list of DMA descriptors to move data to/from peripheral FIFOs.
//!
//! ## Register layout (verified against esp-idf
//! `components/soc/esp32s3/register/soc/gdma_reg.h`)
//!
//! The register file is laid out as a flat array of per-channel blocks with
//! a **per-channel stride of `0xC0`**. `GDMA_IN_CONF0_CH0_REG` is at offset
//! `0x0`, `GDMA_IN_CONF0_CH1_REG` at `0xC0`, etc. Within a channel block the
//! IN (RX) sub-block starts at `+0x00` and the OUT (TX) sub-block at `+0x60`.
//!
//! Per channel `n` (block base = `n * 0xC0`):
//!
//! | Block off | Name              | Notes |
//! |----------:|-------------------|-------|
//! |   0x00    | IN_CONF0          | RX config 0 (bit 4 = MEM_TRANS_EN) |
//! |   0x04    | IN_CONF1          | RX config 1 (R/W round-trip) |
//! |   0x08    | IN_INT_RAW        | bit0 IN_DONE, bit1 IN_SUC_EOF, bit2 IN_ERR_EOF, bit3 IN_DSCR_ERR |
//! |   0x0C    | IN_INT_ST         | RAW & ENA (RO) |
//! |   0x10    | IN_INT_ENA        | per-bit enable (R/W) |
//! |   0x14    | IN_INT_CLR        | W1C of IN_INT_RAW |
//! |   0x20    | IN_LINK           | addr[19:0], stop[21], start[22], restart[23], park[24] (RO=1) |
//! |   0x48    | IN_PERI_SEL       | bits[5:0] peripheral id (R/W); reset = 0x3F (unbound) |
//! |   0x60    | OUT_CONF0         | TX config 0 (R/W round-trip) |
//! |   0x64    | OUT_CONF1         | TX config 1 (R/W round-trip) |
//! |   0x68    | OUT_INT_RAW       | bit0 OUT_DONE, bit1 OUT_EOF, bit2 OUT_DSCR_ERR, bit3 OUT_TOTAL_EOF |
//! |   0x6C    | OUT_INT_ST        | RAW & ENA (RO) |
//! |   0x70    | OUT_INT_ENA       | per-bit enable (R/W) |
//! |   0x74    | OUT_INT_CLR       | W1C of OUT_INT_RAW |
//! |   0x80    | OUT_LINK          | addr[19:0], stop[20], start[21], restart[22], park[23] (RO=1) |
//! |   0xA8    | OUT_PERI_SEL      | bits[5:0] peripheral id (R/W); reset = 0x3F (unbound) |
//!
//! Global: `MISC_CONF` at absolute offset `0x3C8` (R/W round-trip).
//!
//! ## Interrupt sources (esp-idf `soc/esp32s3/include/soc/interrupts.h`)
//!
//! The interrupt-matrix source enum starts at 0 (`ETS_WIFI_MAC=0`), with
//! known anchors `ETS_LEDC=35`, `ETS_RMT=40`. Counting forward,
//! `ETS_DMA_IN_CH0_INTR_SOURCE = 66`, and the ten DMA sources are
//! contiguous: IN_CH0..IN_CH4 = 66..70, then OUT_CH0..OUT_CH4 = 71..75.
//! This peripheral emits source `base + n` for channel `n`'s IN line and
//! `base + 5 + n` for its OUT line, where `base` is the `dma_in_ch0_source`
//! constructor argument (66 on real ESP32-S3).
//!
//! ## Descriptor format (ESP32-S3 TRM §3.4.2 "Linked List Descriptor")
//!
//! Each descriptor is three 32-bit words in RAM (little-endian):
//!
//! | Word | Bits    | Name    | Notes |
//! |-----:|---------|---------|-------|
//! | dw0  | 31      | owner   | 1=DMA owns, 0=CPU; model skips owner=0 descriptors |
//! | dw0  | 30      | suc_eof | TX: last descriptor in chain; RX: set by HW on last |
//! | dw0  | 23:12   | length  | Bytes actually in buffer (TX) or capacity used (RX) |
//! | dw0  | 11:0    | size    | Buffer capacity in bytes |
//! | dw1  |         | buffer  | Full 32-bit bus address of the data buffer |
//! | dw2  |         | next    | Full 32-bit address of next descriptor, or 0 = EOL |
//!
//! ## Memory-to-memory (MEM_TRANS_EN) transfers — what is modelled
//!
//! When bit 4 (`MEM_TRANS_EN`) of `IN_CONF0` is set and both `OUT_LINK` and
//! `IN_LINK` receive a `START` write, the model performs a real descriptor
//! walk and byte copy via `tick_with_bus`:
//!
//! 1. Walk the OUT (TX) descriptor chain, reading bytes from each buffer.
//! 2. Walk the IN (RX) descriptor chain, writing bytes into each buffer.
//! 3. Set `IN_SUC_EOF | IN_DONE` in `IN_INT_RAW` once all bytes are written.
//! 4. Set `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE` in `OUT_INT_RAW`.
//!
//! Descriptors whose `owner` bit is 0 (CPU-owned) are skipped; the walk
//! stops at the first CPU-owned descriptor or at `next == 0`.
//!
//! ## Peripheral-coupled mode — routing split
//!
//! When `MEM_TRANS_EN` (bit 4 of `IN_CONF0`) is **clear**, the link-start
//! path consults `IN_PERI_SEL` / `OUT_PERI_SEL` to decide how to proceed:
//!
//! **Coupled set — real byte movement** (`Uhci0` = UART DMA, `Spi2`,
//! `Spi3`, `I2s0`, `I2s1`, plus `LcdCam` on the **OUT** direction only):
//! the direction is marked `pending_coupled`;
//! `needs_bus_tick` returns `true`; byte movement runs inside
//! `tick_with_bus` via the per-peripheral pumps. For a coupled direction
//! whose pump cannot make progress (e.g. the I2S START bit is clear, or
//! `SPI_CMD.USR` was never kicked) EOF stays **unlatched** — the transfer
//! visibly stalls rather than silently auto-completing.
//!
//! **Fallback set — auto-complete, no byte movement** (explicitly: `Aes`,
//! `Sha`, `AdcDac`, `Rmt`, `LcdCam`, `Unknown`, and the reset / unbound
//! value `0x3F`): the legacy behaviour is preserved — writing
//! `OUTLINK_START` latches `OUT_EOF + OUT_TOTAL_EOF + OUT_DONE`; writing
//! `INLINK_START` latches `IN_SUC_EOF + IN_DONE` — so firmware polling EOF
//! makes forward progress. Firmware that never writes `PERI_SEL` gets
//! `0x3F` (unbound → `Unknown`) and falls through here, preserving full
//! backwards compatibility. `LcdCam` is in this set for the **IN**
//! direction only (camera RX is still unmodelled); its **OUT** direction
//! is coupled and streams pixel words into the LCD_CAM i80 master — see
//! `pump_lcd`. With no `lcd_cam` peripheral on the bus that pump falls
//! back to auto-complete, so a chip config without the block behaves
//! exactly as before.
//!
//! ## Coupled-mode data movement — shared mechanics
//!
//! All coupled pumps share these mechanics:
//!
//! - **Incremental pumping:** at most `COUPLED_BYTES_PER_TICK` (64) bytes
//!   move per `tick_with_bus` call per transfer. Larger transfers resume
//!   across ticks from per-direction walk state (`coupled_desc_ptr`,
//!   `coupled_buf_offset`, `coupled_bytes_moved`) rather than restarting.
//! - **Owner / LEN writeback:** on completing an IN (RX) descriptor (and
//!   in the one-shot M2M walks above) the engine unconditionally writes
//!   dw0 back with the owner bit cleared and dw0[23:12] replaced with the
//!   actual received byte count (stale CPU-seeded values are cleared
//!   first). OUT (TX) descriptors keep their CPU-seeded length and get the
//!   owner-clearing writeback **only when `OUT_AUTO_WRBACK` (bit 2 of
//!   `OUT_CONF0`) is set** — matching silicon, where IN writeback is
//!   always-on but OUT-side owner clearing is opt-in. With the bit clear
//!   (the reset value) a completed OUT chain keeps owner=1 everywhere, so
//!   firmware may legally re-kick OUTLINK_START on the same pre-armed
//!   chain without rewriting dw0.
//! - **Coupling mechanism per peripheral:** UART (UHCI0) couples through
//!   UART0's real MMIO FIFO at offset 0x00 — DMA-written bytes take the
//!   identical path as CPU writes, so serial output, STATUS counts, and
//!   UART interrupts behave the same. SPI2/3 and I2S0/1 have no MMIO
//!   data-port register, so their pumps use the temporary-swap idiom (see
//!   the next section). LCD_CAM likewise has no CPU-visible pixel FIFO and
//!   uses the same swap idiom, at a larger per-tick budget
//!   ([`LCD_BYTES_PER_TICK`]) because a display frame dwarfs a serial
//!   burst.
//! - **EOF policies:** OUT latches `OUT_EOF + OUT_TOTAL_EOF + OUT_DONE`
//!   when its chain drains. UART IN latches `IN_DONE` per filled
//!   descriptor and `IN_SUC_EOF` on chain completion or FIFO-idle after
//!   ≥1 byte (see `pump_uart_in`). SPI IN latches EOF when the
//!   transaction's byte count is exhausted. I2S IN honours `RXEOF_NUM`,
//!   which on the S3 is a **byte** count (ESP-IDF `i2s_ll_rx_set_eof_num`
//!   writes the byte length directly; only the classic ESP32 register
//!   counts words): `IN_SUC_EOF` latches after exactly `RXEOF_NUM` bytes.
//! - **Documented simplification:** when an IN descriptor chain is
//!   exhausted before the data source is (chain under-provisioned), the
//!   model latches `IN_SUC_EOF` and drops the excess, where real silicon
//!   would raise `IN_DSCR_EMPTY`. Firmware sized per ESP-IDF driver
//!   conventions never hits this path.
//!
//! ## SPI2/3 coupling mechanism — design decision
//!
//! GDMA and the GP-SPI controllers are separate peripherals on the bus and
//! the byte handoff cannot ride MMIO: the SPI `W0..W15` buffer is the CPU
//! (non-DMA) data path, and the GP-SPI block has no FIFO data-port register
//! (unlike UART0's FIFO at offset 0x00 that the UHCI0 pump uses above).
//! `PeripheralTickResult::dma_requests` (the STM32 DMA pattern) was
//! evaluated first and rejected: it expresses flat src→dst byte copies
//! issued from a bus-less `tick()` and executed afterwards by the bus, so
//! it can neither walk descriptor chains (which needs bus reads mid-walk)
//! nor obtain MISO bytes from the SPI's attached-device model. Instead the
//! SPI pump uses the same temporary-swap idiom the bus itself uses to lend
//! `&mut self` into `tick_with_bus` (see `bus/mod.rs`): downcast the bus to
//! `SystemBus`, swap the `Esp32s3Spi` instance out behind a stub, exchange
//! one burst of wire bytes via `Esp32s3Spi::dma_transfer`, swap it back.
//! TX and RX may be bound on *different* GDMA channels (ESP-IDF's
//! `gdma_new_channel` allocates them independently), so the pump pairs the
//! OUT and IN directions by PERI_SEL value, not by channel index.
//! The I2S0/I2S1 pump reuses the same swap idiom and PERI_SEL pairing —
//! the I2S block likewise has no MMIO data-port register (samples are
//! DMA-only on the S3).

use crate::peripherals::esp32s3::gpspi::Esp32s3Spi;
use crate::peripherals::esp32s3::i2s::Esp32s3I2s;
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// Number of GDMA channels on the ESP32-S3.
const NUM_CHANNELS: usize = 5;

/// Per-channel register-block stride (`GDMA_IN_CONF0_CH1 - GDMA_IN_CONF0_CH0`).
const CHANNEL_STRIDE: u64 = 0xC0;

/// Absolute offset of the global `GDMA_MISC_CONF_REG`.
const MISC_CONF_OFFSET: u64 = 0x3C8;

// ── IN (RX) sub-block offsets within a channel block ──
const IN_CONF0: u64 = 0x00;
const IN_CONF1: u64 = 0x04;
const IN_INT_RAW: u64 = 0x08;
const IN_INT_ST: u64 = 0x0C;
const IN_INT_ENA: u64 = 0x10;
const IN_INT_CLR: u64 = 0x14;
const IN_LINK: u64 = 0x20;
/// GDMA_IN_PERI_SEL_CHn: binds IN direction to a peripheral.
/// Offset verified against esp-idf gdma_struct.h + esp-pacs esp32s3 DMA PAC.
const IN_PERI_SEL: u64 = 0x48;

// ── OUT (TX) sub-block offsets within a channel block ──
const OUT_CONF0: u64 = 0x60;
const OUT_CONF1: u64 = 0x64;
const OUT_INT_RAW: u64 = 0x68;
const OUT_INT_ST: u64 = 0x6C;
const OUT_INT_ENA: u64 = 0x70;
const OUT_INT_CLR: u64 = 0x74;
const OUT_LINK: u64 = 0x80;
/// GDMA_OUT_PERI_SEL_CHn: binds OUT direction to a peripheral.
/// Offset verified against esp-idf gdma_struct.h + esp-pacs esp32s3 DMA PAC.
const OUT_PERI_SEL: u64 = 0xA8;

// ── IN interrupt bits (IN_INT_*_CH*) ──
const IN_DONE_BIT: u32 = 1 << 0;
const IN_SUC_EOF_BIT: u32 = 1 << 1;
#[allow(dead_code)]
const IN_ERR_EOF_BIT: u32 = 1 << 2;
#[allow(dead_code)]
const IN_DSCR_ERR_BIT: u32 = 1 << 3;

// ── OUT interrupt bits (OUT_INT_*_CH*) ──
const OUT_DONE_BIT: u32 = 1 << 0;
const OUT_EOF_BIT: u32 = 1 << 1;
#[allow(dead_code)]
const OUT_DSCR_ERR_BIT: u32 = 1 << 2;
const OUT_TOTAL_EOF_BIT: u32 = 1 << 3;

// ── IN_LINK (0x20) bit positions ──
const IN_LINK_ADDR_MASK: u32 = 0x000F_FFFF;
const IN_LINK_STOP_BIT: u32 = 1 << 21;
const IN_LINK_START_BIT: u32 = 1 << 22;
const IN_LINK_RESTART_BIT: u32 = 1 << 23;
const IN_LINK_PARK_BIT: u32 = 1 << 24;

// ── OUT_LINK (0x80) bit positions ──
const OUT_LINK_ADDR_MASK: u32 = 0x000F_FFFF;
const OUT_LINK_STOP_BIT: u32 = 1 << 20;
const OUT_LINK_START_BIT: u32 = 1 << 21;
const OUT_LINK_RESTART_BIT: u32 = 1 << 22;
const OUT_LINK_PARK_BIT: u32 = 1 << 23;

// ── IN_CONF0 bit positions ──
/// MEM_TRANS_EN (bit 4): selects memory-to-memory mode on this channel.
const MEM_TRANS_EN_BIT: u32 = 1 << 4;

// ── OUT_CONF0 bit positions ──
/// OUT_AUTO_WRBACK (bit 2): when set, the engine clears the owner bit on
/// each fully consumed OUT (TX) descriptor; when clear (the reset value)
/// OUT descriptors are left untouched, so firmware may re-kick
/// OUTLINK_START on the same pre-armed chain. Bit index verified against
/// the vendored ESP-IDF headers (PlatformIO
/// `framework-arduinoespressif32-libs/esp32s3/include/soc/esp32s3/register/soc/`):
/// `gdma_reg.h` `GDMA_OUT_AUTO_WRBACK_CHn` = `BIT(2)`, R/W, default 0, and
/// `gdma_struct.h` `out.conf0.out_auto_wrback` at bitpos [2]; ESP-IDF
/// drivers enable it via `gdma_ll_tx_enable_auto_write_back`
/// (`hal/gdma_ll.h`). IN (RX) writeback has no such gate on silicon and
/// stays unconditional in the model.
const OUT_AUTO_WRBACK_BIT: u32 = 1 << 2;

/// 6-bit mask for PERI_SEL fields; bits [31:6] are reserved.
const PERI_SEL_MASK: u32 = 0x3F;

/// Reset value for IN/OUT_PERI_SEL: no peripheral bound ("unbound").
/// This value is preserved across reset and keeps the legacy auto-complete
/// behaviour for firmware that never writes PERI_SEL.
const PERI_SEL_RESET: u32 = 0x3F;

/// Peripheral targets GDMA can couple to.
///
/// Values are per the ESP32-S3 TRM / `gdma_struct.h` PERI_IN_SEL / PERI_OUT_SEL
/// encoding verified in `docs/esp32s3_gdma_peri_sel.md` (Task 0 ground truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DmaPeripheral {
    /// GP-SPI2 master/slave (sel = 0)
    Spi2,
    /// GP-SPI3 master/slave (sel = 1)
    Spi3,
    /// UHCI0 bridge → UART DMA path (sel = 2)
    Uhci0,
    /// I2S0 TX/RX (sel = 3)
    I2s0,
    /// I2S1 TX/RX (sel = 4)
    I2s1,
    /// LCD/camera controller (sel = 5) — deferred, fallback behaviour
    LcdCam,
    /// AES accelerator (sel = 6) — fallback auto-complete
    Aes,
    /// SHA accelerator (sel = 7) — fallback auto-complete
    Sha,
    /// SAR ADC (sel = 8) — fallback auto-complete
    AdcDac,
    /// RMT controller (sel = 9) — fallback auto-complete
    Rmt,
    /// Unrecognised or unbound (0x3F = reset / no selection, others unknown)
    Unknown(u32),
}

impl DmaPeripheral {
    fn from_sel(v: u32) -> Self {
        match v & PERI_SEL_MASK {
            0 => Self::Spi2,
            1 => Self::Spi3,
            2 => Self::Uhci0,
            3 => Self::I2s0,
            4 => Self::I2s1,
            5 => Self::LcdCam,
            6 => Self::Aes,
            7 => Self::Sha,
            8 => Self::AdcDac,
            9 => Self::Rmt,
            other => Self::Unknown(other),
        }
    }

    /// True when this peripheral is in the "coupled set" — byte movement is
    /// handled by `tick_with_bus` (Tasks 2–4 fill in the implementations).
    fn is_coupled(self) -> bool {
        matches!(
            self,
            Self::Spi2 | Self::Spi3 | Self::Uhci0 | Self::I2s0 | Self::I2s1
        )
    }
}

/// High-address prefix added to the 20-bit LINK_ADDR field to form a full
/// 32-bit bus address. On ESP32-S3, GDMA descriptors must reside in internal
/// SRAM which is mapped at 0x3FC0_0000; the INLINK/OUTLINK registers carry
/// only bits [19:0] of the descriptor address and the upper 12 bits are
/// implicitly `0x3FC`. This matches the linker-assigned DRAM range and the
/// firmware `LINK_ADDR_MASK = 0x000F_FFFF` masking seen in the Tier-1
/// fixture (and in ESP-IDF drivers).
const DRAM_ADDR_PREFIX: u32 = 0x3FC0_0000;

/// Maximum number of descriptor hops per channel walk (safety guard against
/// infinite loops in corrupted descriptor chains).
const MAX_DESC_CHAIN: usize = 4096;

/// Descriptor dw0 bit positions.
const DESC_OWNER_BIT: u32 = 1 << 31;
/// Descriptor dw0 `suc_eof` (bit 30) — the last descriptor of a transfer.
///
/// The LCD driver keeps a fixed descriptor pool and only rewrites the leading
/// nodes for each transfer, so the trailing nodes still carry the previous
/// (larger) transfer's lengths. `suc_eof` is what tells silicon where the chain
/// really ends; a walk that only checks `next != 0` would stream stale bytes.
const DESC_SUC_EOF_BIT: u32 = 1 << 30;
/// Descriptor dw0 "length" field (bits [23:12]) — bytes valid in the buffer.
/// On IN (RX) descriptors the hardware writes this field on completion; the
/// writeback must CLEAR it first so stale CPU-seeded values don't OR in.
const DESC_LEN_MASK: u32 = 0xFFF << 12;

// ── UHCI0 / UART0 coupling constants ─────────────────────────────────────
//
// UHCI0 bridges GDMA to UART0 by default (ESP-IDF `uart_ll.h` / TRM §26).
// `DR_REG_UART0_BASE = 0x6000_0000` (verified: uart.rs module doc + xtensa.rs
// `configure_xtensa_esp32s3` registration).
//
// FIFO register: offset 0x00 — W pushes a TX byte; R pops one RX byte.
// STATUS register: offset 0x1C — RXFIFO_CNT[9:0] (bits 9:0),
//                                 TXFIFO_CNT[9:0] (bits 25:16).
const UART0_BASE: u64 = 0x6000_0000;
const UART0_FIFO_ADDR: u64 = UART0_BASE; // offset 0x00
const UART0_STATUS_ADDR: u64 = UART0_BASE + 0x1C;

/// STATUS register bit masks.
const UART_RXFIFO_CNT_MASK: u32 = 0x3FF; // bits [9:0]
/// TXFIFO_CNT[25:16] shift — `pump_uart_out` reads this field to compute
/// the available TX FIFO space (back-pressure).
const UART_TXFIFO_CNT_SHIFT: u32 = 16;
const UART_TXFIFO_CNT_MASK: u32 = 0x3FF; // 10-bit field

/// Hardware FIFO depth for ESP32-S3 (`SOC_UART_FIFO_LEN = 128`).
/// `pump_uart_out` caps each tick's writes at `UART_FIFO_LEN - TXFIFO_CNT`
/// so the TX FIFO never overflows; when it is full the pump backs off
/// until the baud-rate drain frees space.
const UART_FIFO_LEN: u32 = 128;

// ── SPI2/3 coupled DMA constants ─────────────────────────────────────────
/// Registered name for GP-SPI2 in the system bus (real base `0x6002_4000`).
const SPI2_S3_NAME: &str = "spi2_s3";
/// Registered name for GP-SPI3 in the system bus (real base `0x6002_5000`).
const SPI3_S3_NAME: &str = "spi3_s3";

// ── I2S0/1 coupled DMA constants ─────────────────────────────────────────
/// Registered name for I2S0 in the system bus (real base `0x6000_F000`).
const I2S0_S3_NAME: &str = "i2s0_s3";
/// Registered name for I2S1 in the system bus (real base `0x6002_D000`).
const I2S1_S3_NAME: &str = "i2s1_s3";

// ── LCD_CAM coupled DMA constants ────────────────────────────────────────
/// Registered name for LCD_CAM in the system bus (real base `0x6004_1000`).
const LCD_CAM_S3_NAME: &str = "lcd_cam";

/// Maximum payload bytes handed to LCD_CAM per `tick_with_bus` call.
///
/// Larger than [`COUPLED_BYTES_PER_TICK`] because a display frame is two
/// orders of magnitude bigger than a UART / SPI burst (a 320×200 RGB565 push
/// is 128 000 bytes) and, unlike those buses, nothing on the far side applies
/// back-pressure — the panel model latches every word it is given. The bound
/// exists only to keep one tick's work finite.
const LCD_BYTES_PER_TICK: usize = 4096;

/// Maximum bytes transferred per `tick_with_bus` call for a coupled channel.
///
/// Bounds latency per tick to a realistic burst size. 64 bytes matches the
/// typical DMA burst used by ESP-IDF `uart_ll.h` (half the 128-deep FIFO)
/// and keeps the simulation engine responsive on long transfers.
const COUPLED_BYTES_PER_TICK: usize = 64;

/// One decoded GDMA linked-list descriptor (TRM §3.4.2) — the single home
/// for descriptor FORMAT knowledge (word layout, field extraction, owner /
/// LEN writeback). Walk and pump POLICIES (per-tick budgets, FIFO
/// backpressure, idle-EOF rules) stay with their callers.
#[derive(Debug, Clone, Copy)]
struct Desc {
    /// Raw first word: owner (bit 31), suc_eof (bit 30), length, size.
    dw0: u32,
    /// dw0[23:12] — bytes valid in the buffer (TX direction reads these).
    len: u32,
    /// dw0[11:0] — buffer capacity in bytes (RX direction fills up to this).
    size: u32,
    /// dw1 — full 32-bit bus address of the data buffer.
    buf: u64,
    /// dw2 — full 32-bit address of the next descriptor; 0 = end-of-list.
    next: u64,
}

impl Desc {
    /// Read and decode the three descriptor words at `addr`.
    fn read(bus: &mut dyn Bus, addr: u64) -> Self {
        let dw0 = bus.read_u32(addr).unwrap_or(0);
        Self {
            dw0,
            len: (dw0 >> 12) & 0xFFF,
            size: dw0 & 0xFFF,
            buf: bus.read_u32(addr + 4).unwrap_or(0) as u64,
            next: bus.read_u32(addr + 8).unwrap_or(0) as u64,
        }
    }

    /// True when the DMA engine owns this descriptor (dw0 bit 31).
    fn dma_owned(&self) -> bool {
        self.dw0 & DESC_OWNER_BIT != 0
    }

    /// True when this descriptor is flagged as the last of a transfer
    /// (dw0 `suc_eof`, bit 30).
    fn suc_eof(&self) -> bool {
        self.dw0 & DESC_SUC_EOF_BIT != 0
    }

    /// Return the descriptor to the CPU: write dw0 back to `addr` with the
    /// owner bit cleared — matching the ESP32-S3 TRM §3.4 hardware
    /// behaviour. RX (IN) descriptors additionally replace the LEN field
    /// [23:12] with the received byte count (`rx_len = Some(n)`, clearing
    /// stale CPU-seeded values first); TX (OUT) descriptors keep their
    /// CPU-seeded length (`rx_len = None`). Callers gate the TX (OUT)
    /// writeback on `OUT_AUTO_WRBACK` (see `OUT_AUTO_WRBACK_BIT`); the
    /// RX (IN) writeback is unconditional, as on silicon.
    fn write_back_owner(&self, bus: &mut dyn Bus, addr: u64, rx_len: Option<u32>) {
        let dw0 = match rx_len {
            Some(n) => (self.dw0 & !(DESC_OWNER_BIT | DESC_LEN_MASK)) | (n << 12),
            None => self.dw0 & !DESC_OWNER_BIT,
        };
        let _ = bus.write_u32(addr, dw0);
    }
}

/// One direction (IN or OUT) of a GDMA channel.
#[derive(Debug, Clone, Copy)]
struct DmaDir {
    conf0: u32,
    conf1: u32,
    /// Latched descriptor-list base address (bits[19:0] of the LINK reg).
    link_addr: u32,
    /// INT_RAW — sticky pending bits, cleared only by INT_CLR (W1C).
    int_raw: u32,
    /// INT_ENA — per-bit IRQ enable.
    int_ena: u32,
    /// PERI_SEL register value (6-bit masked; reset = 0x3F = unbound).
    peri_sel: u32,
    /// Set when this direction has been started in coupled mode (PERI_SEL
    /// names a coupled peripheral and MEM_TRANS_EN is clear). Cleared by
    /// `tick_with_bus` once the transfer completes (EOF latched).
    pending_coupled: bool,
    // ── Incremental coupled-walk state ──────────────────────────────────
    // These fields track progress across multiple `tick_with_bus` calls so
    // that a transfer larger than COUPLED_BYTES_PER_TICK resumes rather
    // than restarting from the head of the descriptor chain each tick.
    // M2M (one-shot) walks ignore these fields entirely.
    //
    /// Current descriptor address for the incremental walk (0 = not started).
    coupled_desc_ptr: u64,
    /// Byte offset within the current descriptor's buffer (OUT direction:
    /// how many bytes of this descriptor have been consumed; IN direction:
    /// how many bytes have been written into this descriptor so far).
    coupled_buf_offset: u32,
    /// Total bytes moved by the current coupled transfer (reset at link
    /// start). The I2S IN pump compares this against the controller's
    /// `RXEOF_NUM` byte count to decide when `IN_SUC_EOF` latches.
    coupled_bytes_moved: u32,
}

impl Default for DmaDir {
    fn default() -> Self {
        Self {
            conf0: 0,
            conf1: 0,
            link_addr: 0,
            int_raw: 0,
            int_ena: 0,
            peri_sel: PERI_SEL_RESET,
            pending_coupled: false,
            coupled_desc_ptr: 0,
            coupled_buf_offset: 0,
            coupled_bytes_moved: 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Channel {
    rx: DmaDir,
    tx: DmaDir,
    /// True when IN_LINK received a START while MEM_TRANS_EN was set.
    in_started: bool,
    /// True when OUT_LINK received a START while MEM_TRANS_EN was set.
    out_started: bool,
    /// True when both `in_started` and `out_started` are set (i.e. both
    /// INLINK_START and OUTLINK_START have been written with MEM_TRANS_EN
    /// active). The `tick_with_bus` pass reads the OUT descriptor chain,
    /// copies bytes into the IN chain, latches EOF, then clears all flags.
    pending_m2m: bool,
}

/// ESP32-S3 GDMA controller — 5 channels × {IN, OUT}.
#[derive(Debug)]
pub struct Esp32s3Gdma {
    channels: [Channel; NUM_CHANNELS],
    /// `GDMA_MISC_CONF_REG` (round-tripped only).
    misc_conf: u32,
    /// Interrupt-matrix source ID for IN channel 0 (66 on real silicon).
    /// IN_CHn = base + n; OUT_CHn = base + 5 + n.
    dma_in_ch0_source: u32,

    /// Bus-published cycle clock (walk-free level export).
    clock: Option<CycleClock>,
}

impl Esp32s3Gdma {
    /// `dma_in_ch0_source` is the interrupt-matrix source ID bound to RX
    /// channel 0 (`ETS_DMA_IN_CH0_INTR_SOURCE` = 66 on ESP32-S3). The other
    /// nine DMA lines are derived contiguously from it.
    pub fn new(dma_in_ch0_source: u32) -> Self {
        Self {
            channels: [Channel::default(); NUM_CHANNELS],
            misc_conf: 0,
            dma_in_ch0_source,

            clock: None,
        }
    }

    /// Decode an absolute window offset into `(channel_index, block_offset)`
    /// for offsets that fall inside a per-channel block. Returns `None` for
    /// the global region (e.g. MISC_CONF) or out-of-range offsets.
    fn channel_of(offset: u64) -> Option<(usize, u64)> {
        let ch = (offset / CHANNEL_STRIDE) as usize;
        if ch >= NUM_CHANNELS {
            return None;
        }
        Some((ch, offset % CHANNEL_STRIDE))
    }

    fn read_word(&self, offset: u64) -> u32 {
        if offset == MISC_CONF_OFFSET {
            return self.misc_conf;
        }
        let Some((ch, blk)) = Self::channel_of(offset) else {
            return 0;
        };
        let c = &self.channels[ch];
        match blk {
            IN_CONF0 => c.rx.conf0,
            IN_CONF1 => c.rx.conf1,
            IN_INT_RAW => c.rx.int_raw,
            IN_INT_ST => c.rx.int_raw & c.rx.int_ena,
            IN_INT_ENA => c.rx.int_ena,
            // IN_INT_CLR is W1C/write-only; reads as 0.
            IN_INT_CLR => 0,
            // PARK bit (24) reads 1 when the channel is idle (not actively
            // walking a list). We model transfers as instantaneous, so the
            // channel is always parked; START self-clears immediately.
            IN_LINK => (c.rx.link_addr & IN_LINK_ADDR_MASK) | IN_LINK_PARK_BIT,
            IN_PERI_SEL => c.rx.peri_sel & PERI_SEL_MASK,
            OUT_CONF0 => c.tx.conf0,
            OUT_CONF1 => c.tx.conf1,
            OUT_INT_RAW => c.tx.int_raw,
            OUT_INT_ST => c.tx.int_raw & c.tx.int_ena,
            OUT_INT_ENA => c.tx.int_ena,
            OUT_INT_CLR => 0,
            OUT_LINK => (c.tx.link_addr & OUT_LINK_ADDR_MASK) | OUT_LINK_PARK_BIT,
            OUT_PERI_SEL => c.tx.peri_sel & PERI_SEL_MASK,
            _ => {
                crate::census_reg!("esp32s3.gdma:Esp32s3Gdma", blk, "read");
                0
            }
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if offset == MISC_CONF_OFFSET {
            self.misc_conf = value;
            return;
        }
        let Some((ch, blk)) = Self::channel_of(offset) else {
            return;
        };
        let c = &mut self.channels[ch];
        match blk {
            IN_CONF0 => c.rx.conf0 = value,
            IN_CONF1 => c.rx.conf1 = value,
            // INT_RAW is R/WTC (write-to-clear via the CLR register); ignore
            // direct writes to RAW, matching silicon's CLR-driven model.
            IN_INT_RAW => {}
            // INT_ST is RO.
            IN_INT_ST => {}
            IN_INT_ENA => c.rx.int_ena = value,
            IN_INT_CLR => {
                // W1C: clear the matching IN_INT_RAW bits.
                c.rx.int_raw &= !value;
            }
            IN_LINK => {
                c.rx.link_addr = value & IN_LINK_ADDR_MASK;
                // INLINK_START: kick the RX channel.
                if value & (IN_LINK_START_BIT | IN_LINK_RESTART_BIT) != 0 {
                    if c.rx.conf0 & MEM_TRANS_EN_BIT != 0 {
                        // MEM_TRANS_EN: track that IN_LINK has been started.
                        // The actual copy runs in tick_with_bus once both
                        // IN_LINK and OUT_LINK have been kicked (the firmware
                        // may start them in either order).
                        c.in_started = true;
                        if c.out_started {
                            c.pending_m2m = true;
                        }
                    } else {
                        // Peripheral-coupled mode: route by PERI_SEL.
                        match DmaPeripheral::from_sel(c.rx.peri_sel) {
                            p if p.is_coupled() => {
                                // Coupled set (SPI2/3, UHCI0, I2S0/1): mark
                                // pending — byte movement runs in tick_with_bus.
                                // Initialise the incremental walk state from the
                                // just-latched link_addr so tick_with_bus starts
                                // at the head of the descriptor chain.
                                c.rx.pending_coupled = true;
                                c.rx.coupled_desc_ptr = Self::full_desc_addr(c.rx.link_addr);
                                c.rx.coupled_buf_offset = 0;
                                c.rx.coupled_bytes_moved = 0;
                            }
                            _ => {
                                // Fallback set (AES, SHA, ADC, RMT, LCD_CAM,
                                // Unknown including unbound 0x3F): auto-complete
                                // so firmware polling IN_SUC_EOF makes forward
                                // progress. Preserves all legacy behaviour for
                                // firmware that never writes PERI_SEL.
                                c.rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
                            }
                        }
                    }
                }
                // STOP: not modelled — in-flight coupled transfers run to
                // completion (or stall visibly). SPI-side aborts are handled
                // by SPI_SOFT_RESET clearing the pending transaction (the
                // stalled GDMA direction then stays visibly pending).
                let _ = IN_LINK_STOP_BIT;
            }
            IN_PERI_SEL => c.rx.peri_sel = value & PERI_SEL_MASK,
            OUT_CONF0 => c.tx.conf0 = value,
            OUT_CONF1 => c.tx.conf1 = value,
            OUT_INT_RAW => {}
            OUT_INT_ST => {}
            OUT_INT_ENA => c.tx.int_ena = value,
            OUT_INT_CLR => {
                c.tx.int_raw &= !value;
            }
            OUT_LINK => {
                c.tx.link_addr = value & OUT_LINK_ADDR_MASK;
                // OUTLINK_START: kick the TX channel.
                if value & (OUT_LINK_START_BIT | OUT_LINK_RESTART_BIT) != 0 {
                    if c.rx.conf0 & MEM_TRANS_EN_BIT != 0 {
                        // MEM_TRANS_EN: track that OUT_LINK has been started.
                        // Set pending_m2m when both sides are ready.
                        c.out_started = true;
                        if c.in_started {
                            c.pending_m2m = true;
                        }
                    } else {
                        // Peripheral-coupled mode: route by PERI_SEL.
                        match DmaPeripheral::from_sel(c.tx.peri_sel) {
                            // LCD_CAM couples on the OUT direction only: the
                            // i80 / RGB master pulls pixel words out of this
                            // chain. The IN direction (sel 5 = camera RX) is
                            // still unmodelled and keeps the auto-complete
                            // fallback below.
                            DmaPeripheral::LcdCam => {
                                c.tx.pending_coupled = true;
                                c.tx.coupled_desc_ptr = Self::full_desc_addr(c.tx.link_addr);
                                c.tx.coupled_buf_offset = 0;
                                c.tx.coupled_bytes_moved = 0;
                            }
                            p if p.is_coupled() => {
                                // Coupled set: mark pending; byte movement runs
                                // in tick_with_bus. Initialise the incremental
                                // walk state from the just-latched link_addr.
                                c.tx.pending_coupled = true;
                                c.tx.coupled_desc_ptr = Self::full_desc_addr(c.tx.link_addr);
                                c.tx.coupled_buf_offset = 0;
                                c.tx.coupled_bytes_moved = 0;
                            }
                            _ => {
                                // Fallback set: auto-complete.
                                c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                            }
                        }
                    }
                }
                let _ = OUT_LINK_STOP_BIT;
            }
            OUT_PERI_SEL => c.tx.peri_sel = value & PERI_SEL_MASK,
            _ => {
                crate::census_reg!("esp32s3.gdma:Esp32s3Gdma", blk, "write");
            }
        }
    }

    /// Walk an OUT (TX) descriptor chain starting at `desc_addr` and collect
    /// all bytes from the data buffers. Returns the bytes if successful.
    ///
    /// Stops at the first descriptor whose `owner` bit is 0 (CPU-owned),
    /// at `next == 0` (end-of-list), or after `MAX_DESC_CHAIN` hops.
    ///
    /// After consuming each descriptor, if `auto_wrback` is set (the
    /// channel's `OUT_AUTO_WRBACK`, bit 2 of `OUT_CONF0`), writes back
    /// `dw0 & !DESC_OWNER_BIT` to the descriptor address so the CPU sees
    /// `owner=0` — matching the ESP32-S3 TRM §3.4 hardware behaviour. With
    /// `auto_wrback` clear the descriptors are left untouched (silicon
    /// gates OUT-side owner clearing on this bit), so a re-kicked walk
    /// over the same chain transfers the same bytes again. Loop
    /// termination never depended on the owner writeback: the walk stops
    /// at `next == 0`, and the `MAX_DESC_CHAIN` hop bound caps circular
    /// chains (e.g. `next == self`) in both modes.
    fn walk_out_chain(bus: &mut dyn Bus, desc_addr: u64, auto_wrback: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut addr = desc_addr;
        for _ in 0..MAX_DESC_CHAIN {
            if addr == 0 {
                break;
            }
            let d = Desc::read(bus, addr);
            // Skip CPU-owned descriptors (owner=0).
            if !d.dma_owned() {
                break;
            }

            for i in 0..d.len {
                bytes.push(bus.read_u8(d.buf + i as u64).unwrap_or(0));
            }

            // Descriptor returned to CPU (owner cleared) — only when the
            // channel opted in via OUT_AUTO_WRBACK.
            if auto_wrback {
                d.write_back_owner(bus, addr, None);
            }

            if d.next == 0 {
                break;
            }
            addr = d.next;
        }
        bytes
    }

    /// Walk an IN (RX) descriptor chain starting at `desc_addr` and write
    /// `bytes` into the data buffers.
    ///
    /// After writing `to_write` bytes into each descriptor, writes back dw0
    /// with the owner bit cleared and bits [23:12] set to `to_write` so the
    /// CPU sees `owner=0` and the actual received byte count — matching the
    /// ESP32-S3 TRM §3.4 hardware behaviour.
    fn walk_in_chain(bus: &mut dyn Bus, desc_addr: u64, bytes: &[u8]) {
        let mut remaining = bytes;
        let mut addr = desc_addr;
        for _ in 0..MAX_DESC_CHAIN {
            if addr == 0 || remaining.is_empty() {
                break;
            }
            let d = Desc::read(bus, addr);
            // Skip CPU-owned descriptors.
            if !d.dma_owned() {
                break;
            }

            let to_write = remaining.len().min(d.size as usize);
            for (i, &b) in remaining[..to_write].iter().enumerate() {
                let _ = bus.write_u8(d.buf + i as u64, b);
            }
            remaining = &remaining[to_write..];

            // Owner bit cleared, length field set to bytes written.
            d.write_back_owner(bus, addr, Some(to_write as u32));

            if d.next == 0 || remaining.is_empty() {
                break;
            }
            addr = d.next;
        }
    }

    /// Reconstruct the full 32-bit bus address from the 20-bit LINK_ADDR
    /// field. ESP32-S3 GDMA descriptors must reside in internal SRAM
    /// (`0x3FC0_0000`–`0x3FCF_FFFF`); the upper 12 bits are implicit.
    fn full_desc_addr(link_addr_20: u32) -> u64 {
        (DRAM_ADDR_PREFIX | (link_addr_20 & IN_LINK_ADDR_MASK)) as u64
    }

    /// Pump an OUT (TX) coupled UART transfer: walk the descriptor chain and
    /// write bytes into the UART TX FIFO via MMIO.
    ///
    /// Returns `true` when the chain is fully drained (transfer complete).
    ///
    /// **EOF policy (OUT/TX):** `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE` are
    /// latched by the caller once this function returns `true` — i.e. after
    /// the last byte of the last descriptor has entered the FIFO.
    ///
    /// **Throughput bound:** at most `COUPLED_BYTES_PER_TICK` bytes per call,
    /// further limited by the available UART TX FIFO space (`UART_FIFO_LEN −
    /// TXFIFO_CNT`).  When the FIFO is full the pump backs off without
    /// advancing — the engine revisits on the next tick once the baud-rate
    /// drain has freed space.
    ///
    /// **Owner writeback:** gated on the channel's `OUT_AUTO_WRBACK`
    /// (`dir.conf0` bit 2); with the bit clear consumed descriptors stay
    /// DMA-owned so firmware may re-kick the same chain. The walk advances
    /// via `coupled_desc_ptr`, never via the owner bit, so progress and
    /// termination are unchanged either way.
    fn pump_uart_out(dir: &mut DmaDir, bus: &mut dyn Bus) -> bool {
        // Determine available TX FIFO space.
        let status = bus.read_u32(UART0_STATUS_ADDR).unwrap_or(0);
        let tx_in_use = ((status >> UART_TXFIFO_CNT_SHIFT) & UART_TXFIFO_CNT_MASK) as usize;
        let tx_free = (UART_FIFO_LEN as usize).saturating_sub(tx_in_use);

        let mut budget = COUPLED_BYTES_PER_TICK.min(tx_free);

        if budget == 0 {
            // FIFO full; wait for the baud-rate drain to free space.
            return false;
        }

        // Hop bound guards against corrupted (e.g. circular) chains.
        for _ in 0..MAX_DESC_CHAIN {
            let addr = dir.coupled_desc_ptr;
            if addr == 0 || budget == 0 {
                break;
            }

            let d = Desc::read(bus, addr);
            // Skip CPU-owned descriptors; treat as end-of-chain.
            if !d.dma_owned() {
                return true; // chain drained / halted
            }

            // How many bytes remain in this descriptor?
            let remaining = d.len.saturating_sub(dir.coupled_buf_offset) as usize;
            let to_send = remaining.min(budget);

            for i in 0..to_send {
                let byte = bus
                    .read_u8(d.buf + (dir.coupled_buf_offset as u64) + i as u64)
                    .unwrap_or(0);
                let _ = bus.write_u8(UART0_FIFO_ADDR, byte);
            }
            budget -= to_send;
            dir.coupled_buf_offset += to_send as u32;

            if dir.coupled_buf_offset >= d.len {
                // Descriptor fully consumed; returned to CPU (owner
                // cleared) only when OUT_AUTO_WRBACK is set.
                if dir.conf0 & OUT_AUTO_WRBACK_BIT != 0 {
                    d.write_back_owner(bus, addr, None);
                }
                // Advance to next.
                dir.coupled_buf_offset = 0;
                if d.next == 0 {
                    dir.coupled_desc_ptr = 0;
                    return true; // end of chain
                }
                dir.coupled_desc_ptr = d.next;
            } else {
                // Partially consumed (budget or FIFO exhausted); resume next tick.
                break;
            }
        }
        false // not yet done
    }

    /// Pump an IN (RX) coupled UART transfer: read bytes from the UART RX FIFO
    /// via MMIO and write them into the descriptor chain.
    ///
    /// Returns `true` when EOF should be latched.
    ///
    /// **EOF policy (IN/RX):**
    /// - `IN_DONE` is latched per completed descriptor (when its capacity is
    ///   fully written), matching how ESP-IDF `uart_read_bytes` expects the
    ///   DMA engine to signal per-buffer completion.
    /// - `IN_SUC_EOF` is latched when the descriptor chain is fully written
    ///   **OR** when the UART RX FIFO empties after at least one byte has been
    ///   moved — whichever comes first. This mirrors the ESP-IDF
    ///   `uart_intr_handler_default` / UHCI EOF semantics: the driver wakes on
    ///   `IN_SUC_EOF` which fires as soon as the FIFO idle-timeout drains the
    ///   last byte into DMA, even if the descriptor still has spare capacity.
    ///
    /// **Throughput bound:** at most `COUPLED_BYTES_PER_TICK` bytes per call.
    ///
    /// Returns `(eof, in_done_latched)`.
    fn pump_uart_in(dir: &mut DmaDir, bus: &mut dyn Bus) -> (bool, bool) {
        // Read live RXFIFO_CNT from STATUS register.
        let status = bus.read_u32(UART0_STATUS_ADDR).unwrap_or(0);
        let mut rx_avail = (status & UART_RXFIFO_CNT_MASK) as usize;

        if rx_avail == 0 {
            // No bytes available; nothing to do this tick.
            return (false, false);
        }

        let mut budget = COUPLED_BYTES_PER_TICK;
        let mut any_moved = false;
        let mut in_done = false;

        // Hop bound guards against corrupted (e.g. circular) chains.
        for _ in 0..MAX_DESC_CHAIN {
            let addr = dir.coupled_desc_ptr;
            if addr == 0 || budget == 0 || rx_avail == 0 {
                break;
            }

            let d = Desc::read(bus, addr);
            if !d.dma_owned() {
                // CPU-owned: treat as end-of-chain → EOF.
                let eof = any_moved;
                return (eof, in_done);
            }

            let remaining_cap = d.size.saturating_sub(dir.coupled_buf_offset) as usize;
            let to_recv = remaining_cap.min(budget).min(rx_avail);

            for i in 0..to_recv {
                let byte = bus.read_u8(UART0_FIFO_ADDR).unwrap_or(0);
                let _ = bus.write_u8(d.buf + (dir.coupled_buf_offset as u64) + i as u64, byte);
            }
            budget -= to_recv;
            rx_avail -= to_recv;
            dir.coupled_buf_offset += to_recv as u32;
            if to_recv > 0 {
                any_moved = true;
            }

            if dir.coupled_buf_offset >= d.size {
                // Descriptor capacity filled; owner cleared, length = capacity.
                d.write_back_owner(bus, addr, Some(d.size));
                in_done = true;
                dir.coupled_buf_offset = 0;
                if d.next == 0 {
                    dir.coupled_desc_ptr = 0;
                    return (true, true); // chain done → IN_SUC_EOF + IN_DONE
                }
                dir.coupled_desc_ptr = d.next;
            } else {
                // Descriptor partially filled.
                break;
            }
        }

        // Check again after the loop: if the FIFO is now empty and we moved
        // at least one byte, latch IN_SUC_EOF (FIFO-idle EOF).
        let status2 = bus.read_u32(UART0_STATUS_ADDR).unwrap_or(0);
        let rx_remaining = (status2 & UART_RXFIFO_CNT_MASK) as usize;
        let eof = any_moved && rx_remaining == 0;
        // If we produced EOF on a partially-filled descriptor (FIFO-idle path),
        // write back the descriptor dw0 with owner cleared and partial length.
        // `coupled_buf_offset > 0` guards the case where the moved bytes
        // exactly filled the previous descriptor: the current one received
        // nothing and must stay DMA-owned (silicon leaves it untouched).
        if eof && dir.coupled_desc_ptr != 0 && dir.coupled_buf_offset > 0 {
            let addr = dir.coupled_desc_ptr;
            let d = Desc::read(bus, addr);
            if d.dma_owned() {
                d.write_back_owner(bus, addr, Some(dir.coupled_buf_offset));
            }
        }
        (eof, in_done)
    }

    /// Read up to `budget` bytes from an OUT (TX) descriptor chain, resuming
    /// from `dir.coupled_desc_ptr` / `coupled_buf_offset`. Each fully
    /// consumed descriptor gets its owner bit written back to CPU (0) —
    /// but only when the channel's `OUT_AUTO_WRBACK` (`dir.conf0` bit 2)
    /// is set; with the bit clear descriptors stay DMA-owned so firmware
    /// may re-kick the same chain. `coupled_desc_ptr` becomes 0 at
    /// end-of-chain (next == 0 or a CPU-owned descriptor); the walk
    /// advances via `coupled_desc_ptr`, never via the owner bit, and the
    /// `MAX_DESC_CHAIN` hop bound caps circular chains in both modes. May
    /// return fewer bytes than `budget` when the chain is exhausted.
    ///
    /// `stop_at_suc_eof` ends the walk at the first descriptor flagged
    /// `suc_eof` even when `next != 0` — required for peripherals (LCD_CAM)
    /// that reuse a fixed descriptor pool across transfers, harmless but
    /// unnecessary for the ones that rebuild their chain each time.
    fn coupled_out_collect(
        dir: &mut DmaDir,
        bus: &mut dyn Bus,
        budget: usize,
        stop_at_suc_eof: bool,
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(budget);
        // Hop bound guards against corrupted (e.g. circular) chains.
        for _ in 0..MAX_DESC_CHAIN {
            if out.len() >= budget {
                break;
            }
            let addr = dir.coupled_desc_ptr;
            if addr == 0 {
                break;
            }
            let d = Desc::read(bus, addr);
            if !d.dma_owned() {
                // CPU-owned: chain halted here.
                dir.coupled_desc_ptr = 0;
                break;
            }

            let remaining = d.len.saturating_sub(dir.coupled_buf_offset) as usize;
            let to_read = remaining.min(budget - out.len());
            for i in 0..to_read {
                out.push(
                    bus.read_u8(d.buf + dir.coupled_buf_offset as u64 + i as u64)
                        .unwrap_or(0),
                );
            }
            dir.coupled_buf_offset += to_read as u32;

            if dir.coupled_buf_offset >= d.len {
                // Descriptor fully consumed: return it to the CPU — only
                // when OUT_AUTO_WRBACK is set.
                if dir.conf0 & OUT_AUTO_WRBACK_BIT != 0 {
                    d.write_back_owner(bus, addr, None);
                }
                dir.coupled_buf_offset = 0;
                dir.coupled_desc_ptr = if d.next == 0 || (stop_at_suc_eof && d.suc_eof()) {
                    0
                } else {
                    d.next
                };
            }
            // else: budget exhausted mid-descriptor; resume next tick.
        }
        out
    }

    /// Write `bytes` into an IN (RX) descriptor chain, resuming from
    /// `dir.coupled_desc_ptr` / `coupled_buf_offset`. Each filled descriptor
    /// gets owner cleared and the length field [23:12] set to its capacity.
    /// Bytes beyond the end of the chain are dropped (the chain was
    /// under-provisioned; real silicon raises `IN_DSCR_EMPTY` here — a
    /// documented simplification, see the module doc).
    fn coupled_in_write(dir: &mut DmaDir, bus: &mut dyn Bus, bytes: &[u8]) {
        let mut written = 0usize;
        // Hop bound guards against corrupted (e.g. circular) chains.
        for _ in 0..MAX_DESC_CHAIN {
            if written >= bytes.len() {
                break;
            }
            let addr = dir.coupled_desc_ptr;
            if addr == 0 {
                break;
            }
            let d = Desc::read(bus, addr);
            if !d.dma_owned() {
                dir.coupled_desc_ptr = 0;
                break;
            }

            let cap = d.size.saturating_sub(dir.coupled_buf_offset) as usize;
            let n = cap.min(bytes.len() - written);
            for i in 0..n {
                let _ = bus.write_u8(
                    d.buf + dir.coupled_buf_offset as u64 + i as u64,
                    bytes[written + i],
                );
            }
            written += n;
            dir.coupled_buf_offset += n as u32;

            if dir.coupled_buf_offset >= d.size {
                // Capacity filled: owner back to CPU, received length = size.
                d.write_back_owner(bus, addr, Some(d.size));
                dir.coupled_buf_offset = 0;
                dir.coupled_desc_ptr = if d.next == 0 { 0 } else { d.next };
            }
            // else: out of bytes mid-descriptor; resume next tick (or
            // finalize with a partial-length writeback at transaction end).
        }
    }

    /// Finalize an IN direction at transaction end: if the last descriptor
    /// is partially filled, write back owner=0 with the partial length so
    /// drivers polling the owner bit / length field see the real count.
    fn coupled_in_finalize(dir: &mut DmaDir, bus: &mut dyn Bus) {
        if dir.coupled_desc_ptr != 0 && dir.coupled_buf_offset > 0 {
            let addr = dir.coupled_desc_ptr;
            let d = Desc::read(bus, addr);
            if d.dma_owned() {
                d.write_back_owner(bus, addr, Some(dir.coupled_buf_offset));
            }
        }
    }

    /// Service the in-flight DMA transaction (if any) of one GP-SPI
    /// controller. See the module-level "SPI2/3 coupling mechanism" section
    /// for the design rationale.
    ///
    /// Per tick, up to `COUPLED_BYTES_PER_TICK` wire bytes are exchanged:
    /// MOSI bytes come from the OUT chain of whichever channel has
    /// `OUT_PERI_SEL == peri` pending (0xFF idle-high filler when TX-DMA is
    /// disabled or the chain under-runs); each byte is exchanged with the
    /// SPI's attached devices via `dma_transfer`; MISO bytes land in the IN
    /// chain of whichever channel has `IN_PERI_SEL == peri` pending. The
    /// pump stalls (no progress, state retained) until every DMA-enabled
    /// direction has its GDMA link started — firmware may kick `USR` and
    /// the links in either order.
    ///
    /// On the tick that completes the transaction: the SPI latches
    /// TRANS_DONE (`dma_complete`), the TX channel latches
    /// `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE`, the RX channel latches
    /// `IN_SUC_EOF | IN_DONE`, and both directions clear `pending_coupled`.
    fn pump_spi(&mut self, bus: &mut dyn Bus, peri: DmaPeripheral, spi_name: &str) {
        use crate::bus::SystemBus;
        use crate::peripherals::stub::StubPeripheral;

        let tx_idx = self
            .channels
            .iter()
            .position(|c| c.tx.pending_coupled && DmaPeripheral::from_sel(c.tx.peri_sel) == peri);
        let rx_idx = self
            .channels
            .iter()
            .position(|c| c.rx.pending_coupled && DmaPeripheral::from_sel(c.rx.peri_sel) == peri);
        if tx_idx.is_none() && rx_idx.is_none() {
            return;
        }

        let Some(sys_bus) = bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>()) else {
            return;
        };
        let Some(spi_idx) = sys_bus.find_peripheral_index_by_name(spi_name) else {
            return;
        };

        // Swap the SPI out from behind a stub — the same dance the bus uses
        // to lend itself into `tick_with_bus` — so we can hold `&mut` to the
        // SPI and still route descriptor reads/writes through the bus.
        let placeholder: Box<dyn Peripheral> = Box::new(StubPeripheral::new(0));
        let mut spi_dev = std::mem::replace(&mut sys_bus.peripherals[spi_idx].dev, placeholder);

        'work: {
            let Some(spi) = spi_dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Esp32s3Spi>())
            else {
                break 'work;
            };
            let Some(pending) = spi.dma_pending() else {
                // GDMA links started but firmware hasn't kicked SPI_CMD.USR
                // yet — stall, keep pending_coupled so we revisit next tick.
                break 'work;
            };
            // Stall until every DMA-enabled direction has its link started.
            if (pending.tx_ena && tx_idx.is_none()) || (pending.rx_ena && rx_idx.is_none()) {
                break 'work;
            }

            let k =
                COUPLED_BYTES_PER_TICK.min(pending.total_bytes.saturating_sub(pending.transferred));
            if k == 0 {
                // Defensive: a pending record with nothing left to move is
                // corrupt state (dma_complete clears `pending_dma` on the
                // completing tick, so `transferred < total_bytes` holds while
                // pending). Stall visibly rather than spin a zero-byte pump.
                break 'work;
            }
            let mosi = match tx_idx {
                Some(i) if pending.tx_ena => {
                    let mut m = Self::coupled_out_collect(
                        &mut self.channels[i].tx,
                        &mut *sys_bus,
                        k,
                        false,
                    );
                    // OUT chain under-provisioned: the TX FIFO under-runs and
                    // the line idles high for the rest of the burst.
                    m.resize(k, 0xFF);
                    m
                }
                // RX-only DMA transaction: nothing drives MOSI — idle high.
                _ => vec![0xFF; k],
            };
            let miso = spi.dma_transfer(&mosi);
            if let Some(i) = rx_idx {
                if pending.rx_ena {
                    Self::coupled_in_write(&mut self.channels[i].rx, &mut *sys_bus, &miso);
                }
            }

            if pending.transferred + k >= pending.total_bytes {
                // Transaction complete this tick. Only the directions the
                // transaction actually enabled latch EOF — a started link
                // the SPI never used stays pending (visible hang, matching
                // the module's coupled-without-pump philosophy).
                if let Some(i) = rx_idx {
                    if pending.rx_ena {
                        Self::coupled_in_finalize(&mut self.channels[i].rx, &mut *sys_bus);
                        let rx = &mut self.channels[i].rx;
                        rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
                        rx.pending_coupled = false;
                        rx.coupled_desc_ptr = 0;
                        rx.coupled_buf_offset = 0;
                    }
                }
                if let Some(i) = tx_idx {
                    if pending.tx_ena {
                        let tx = &mut self.channels[i].tx;
                        tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                        tx.pending_coupled = false;
                        tx.coupled_desc_ptr = 0;
                        tx.coupled_buf_offset = 0;
                    }
                }
                spi.dma_complete();
            }
        }

        sys_bus.peripherals[spi_idx].dev = spi_dev;
    }

    /// Service GDMA↔I2S coupled sample streaming for one controller.
    ///
    /// I2S on the S3 has no CPU-visible sample FIFO and no MMIO data-port
    /// register, so (like SPI, unlike UART) the byte handoff cannot ride
    /// MMIO: the pump reuses `pump_spi`'s temporary-swap idiom — downcast
    /// the bus to `SystemBus`, lend the `Esp32s3I2s` instance out from
    /// behind a stub, move one burst of bytes, swap it back. TX and RX may
    /// be bound on different GDMA channels, so directions are paired by
    /// PERI_SEL value (I2S0 = 3, I2S1 = 4), not by channel index.
    ///
    /// **TX (OUT):** while the controller's `TX_START` bit is set, up to
    /// `COUPLED_BYTES_PER_TICK` bytes per tick stream from the OUT
    /// descriptor chain into the I2S TX sample sink.
    /// `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE` latch when the chain drains.
    ///
    /// **RX (IN):** while `RX_START` is set, queued I2S RX sample bytes
    /// fill the IN descriptor chain. `RXEOF_NUM` is a **byte** count on the
    /// S3 (ESP-IDF `i2s_ll_rx_set_eof_num` writes the byte length directly;
    /// only the classic ESP32 register counts words): `IN_SUC_EOF |
    /// IN_DONE` latch once exactly `RXEOF_NUM` bytes have been received —
    /// or when the descriptor chain is exhausted, whichever comes first.
    /// `RXEOF_NUM == 0` (reset value; ESP-IDF always programs it before
    /// starting RX) disables the byte-count trigger, leaving only the
    /// chain-exhaustion EOF.
    ///
    /// While the corresponding START bit is clear the pump stalls — no
    /// data movement, walk state retained — and resumes once firmware sets
    /// the bit (the engine keeps revisiting via `needs_bus_tick`).
    fn pump_i2s(&mut self, bus: &mut dyn Bus, peri: DmaPeripheral, i2s_name: &str) {
        use crate::bus::SystemBus;
        use crate::peripherals::stub::StubPeripheral;

        let tx_idx = self
            .channels
            .iter()
            .position(|c| c.tx.pending_coupled && DmaPeripheral::from_sel(c.tx.peri_sel) == peri);
        let rx_idx = self
            .channels
            .iter()
            .position(|c| c.rx.pending_coupled && DmaPeripheral::from_sel(c.rx.peri_sel) == peri);
        if tx_idx.is_none() && rx_idx.is_none() {
            return;
        }

        let Some(sys_bus) = bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>()) else {
            return;
        };
        let Some(i2s_idx) = sys_bus.find_peripheral_index_by_name(i2s_name) else {
            return;
        };

        // Swap the I2S out from behind a stub (same dance as `pump_spi`) so
        // we can hold `&mut` to it while descriptor reads/writes still route
        // through the bus.
        let placeholder: Box<dyn Peripheral> = Box::new(StubPeripheral::new(0));
        let mut i2s_dev = std::mem::replace(&mut sys_bus.peripherals[i2s_idx].dev, placeholder);

        'work: {
            let Some(i2s) = i2s_dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Esp32s3I2s>())
            else {
                break 'work;
            };

            // ── TX: OUT descriptor chain → I2S sample sink ──────────────
            if let Some(i) = tx_idx {
                if i2s.tx_running() {
                    let bytes = Self::coupled_out_collect(
                        &mut self.channels[i].tx,
                        &mut *sys_bus,
                        COUPLED_BYTES_PER_TICK,
                        false,
                    );
                    i2s.dma_push_tx(&bytes);
                    let tx = &mut self.channels[i].tx;
                    if tx.coupled_desc_ptr == 0 {
                        // Chain drained (or halted at a CPU-owned
                        // descriptor): transfer complete.
                        tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                        tx.pending_coupled = false;
                        tx.coupled_buf_offset = 0;
                    }
                }
                // TX_START clear: stall, keep pending_coupled.
            }

            // ── RX: I2S sample source → IN descriptor chain ─────────────
            if let Some(i) = rx_idx {
                if i2s.rx_running() {
                    let rx = &mut self.channels[i].rx;
                    let eof_num = i2s.rxeof_num();
                    // Never consume past the EOF threshold: IN_SUC_EOF
                    // latches after EXACTLY eof_num bytes; excess source
                    // bytes stay queued for the next transfer.
                    let to_eof = if eof_num == 0 {
                        usize::MAX
                    } else {
                        eof_num.saturating_sub(rx.coupled_bytes_moved) as usize
                    };
                    let bytes = i2s.dma_pop_rx(COUPLED_BYTES_PER_TICK.min(to_eof));
                    if !bytes.is_empty() {
                        Self::coupled_in_write(rx, &mut *sys_bus, &bytes);
                        rx.coupled_bytes_moved += bytes.len() as u32;
                    }
                    let eof_reached = eof_num != 0 && rx.coupled_bytes_moved >= eof_num;
                    if eof_reached || rx.coupled_desc_ptr == 0 {
                        Self::coupled_in_finalize(rx, &mut *sys_bus);
                        rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
                        rx.pending_coupled = false;
                        rx.coupled_desc_ptr = 0;
                        rx.coupled_buf_offset = 0;
                    }
                }
                // RX_START clear: stall, keep pending_coupled.
            }
        }

        sys_bus.peripherals[i2s_idx].dev = i2s_dev;
    }

    /// Service GDMA→LCD_CAM pixel streaming (`PERI_SEL == 5`, OUT direction).
    ///
    /// This is the path an `esp_lcd` i80 transaction takes: the driver mounts
    /// the payload on the outlink chain and writes `OUT_LINK.START`
    /// (`gdma_start`), *then* sets `LCD_USER.LCD_START`. So the chain is armed
    /// before the consumer exists, and this pump must stall in between —
    /// exactly as silicon does, with the DMA holding data the LCD has not
    /// clocked out yet.
    ///
    /// Per tick, once `LCD_USER.LCD_START` is set and the DOUT phase is
    /// enabled, up to [`LCD_BYTES_PER_TICK`] bytes move from the descriptor
    /// chain into [`Esp32s3LcdCam::dma_push_tx`], which packs them into bus
    /// words and drives the attached panel. When the chain drains,
    /// `dma_finish` releases the LCD's TRANS_DONE and this channel latches
    /// `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE`.
    ///
    /// The walk stops at `suc_eof` as well as at `next == 0` / a CPU-owned
    /// descriptor: the i80 driver keeps a fixed descriptor pool and rewrites
    /// only the leading nodes per transfer (see [`DESC_SUC_EOF_BIT`]).
    ///
    /// **No LCD_CAM on the bus** (a chip config without it, or a harness):
    /// the transfer auto-completes as it did before LCD coupling existed, so
    /// firmware cannot hang waiting for a peripheral that was never wired.
    fn pump_lcd(&mut self, bus: &mut dyn Bus) {
        use crate::bus::SystemBus;
        use crate::peripherals::esp32s3::lcd_cam::Esp32s3LcdCam;
        use crate::peripherals::stub::StubPeripheral;

        let Some(tx_idx) = self.channels.iter().position(|c| {
            c.tx.pending_coupled && DmaPeripheral::from_sel(c.tx.peri_sel) == DmaPeripheral::LcdCam
        }) else {
            return;
        };

        let Some(sys_bus) = bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>()) else {
            return;
        };
        let complete = |tx: &mut DmaDir| {
            tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
            tx.pending_coupled = false;
            tx.coupled_desc_ptr = 0;
            tx.coupled_buf_offset = 0;
        };
        let Some(lcd_idx) = sys_bus.find_peripheral_index_by_name(LCD_CAM_S3_NAME) else {
            complete(&mut self.channels[tx_idx].tx);
            return;
        };

        // Lend the LCD_CAM out from behind a stub (the `pump_spi` dance) so we
        // can hold `&mut` to it while descriptor reads still route through the
        // bus.
        let placeholder: Box<dyn Peripheral> = Box::new(StubPeripheral::new(0));
        let mut lcd_dev = std::mem::replace(&mut sys_bus.peripherals[lcd_idx].dev, placeholder);

        'work: {
            let Some(lcd) = lcd_dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Esp32s3LcdCam>())
            else {
                complete(&mut self.channels[tx_idx].tx);
                break 'work;
            };

            // Tell the LCD a payload chain is standing by. Must happen even
            // while it stalls below: it is what makes the LCD hold TRANS_DONE
            // back once LCD_START lands.
            lcd.dma_arm();

            if !lcd.dma_wants_data() {
                // Transaction not started yet (or has no DOUT phase): hold the
                // chain, revisit next tick.
                break 'work;
            }

            let bytes = Self::coupled_out_collect(
                &mut self.channels[tx_idx].tx,
                &mut *sys_bus,
                LCD_BYTES_PER_TICK,
                true,
            );
            lcd.dma_push_tx(&bytes);

            if self.channels[tx_idx].tx.coupled_desc_ptr == 0 {
                // Chain drained (end-of-list, suc_eof, or a CPU-owned
                // descriptor): the DOUT phase is over.
                lcd.dma_finish();
                complete(&mut self.channels[tx_idx].tx);
            }
        }

        sys_bus.peripherals[lcd_idx].dev = lcd_dev;
    }

    /// Execute all pending descriptor walks and coupled-mode ticks.
    ///
    /// For each channel with `pending_m2m` set:
    /// 1. Walk the OUT (TX) descriptor chain and collect bytes.
    /// 2. Walk the IN (RX) descriptor chain and write bytes.
    /// 3. Latch `IN_SUC_EOF | IN_DONE` and `OUT_EOF | OUT_TOTAL_EOF |
    ///    OUT_DONE` in the respective INT_RAW registers.
    ///
    /// For UHCI0 (UART DMA) coupled channels with `pending_coupled` set:
    /// - OUT: push bytes from the descriptor chain into the UART TX FIFO.
    ///   `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE` are latched once the chain
    ///   is fully drained; `pending_coupled` is then cleared.
    /// - IN: pop bytes from the UART RX FIFO into the descriptor chain.
    ///   `IN_DONE` is latched per completed descriptor; `IN_SUC_EOF` is
    ///   latched when the chain is fully written or the FIFO idles after
    ///   ≥1 byte moved; `pending_coupled` is cleared on EOF.
    ///
    /// For SPI2/SPI3 coupled channels, `pump_spi` exchanges up to
    /// `COUPLED_BYTES_PER_TICK` wire bytes per tick with the GP-SPI
    /// controller (see its doc comment for the EOF / TRANS_DONE contract).
    ///
    /// For I2S0/I2S1 coupled channels, `pump_i2s` streams up to
    /// `COUPLED_BYTES_PER_TICK` bytes per tick between the descriptor
    /// chains and the controller's sample sink/source, gated by the I2S
    /// TX/RX START bits (see its doc comment for the RXEOF_NUM contract).
    fn do_tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // SPI and I2S pumps run outside the per-channel loop: a transfer's
        // TX and RX directions may live on different channels (paired by
        // PERI_SEL).
        self.pump_spi(bus, DmaPeripheral::Spi2, SPI2_S3_NAME);
        self.pump_spi(bus, DmaPeripheral::Spi3, SPI3_S3_NAME);
        self.pump_i2s(bus, DmaPeripheral::I2s0, I2S0_S3_NAME);
        self.pump_i2s(bus, DmaPeripheral::I2s1, I2S1_S3_NAME);
        self.pump_lcd(bus);

        for (ch_idx, c) in self.channels.iter_mut().enumerate() {
            // ── UHCI0 (UART) coupled OUT (TX) ────────────────────────────
            if c.tx.pending_coupled
                && DmaPeripheral::from_sel(c.tx.peri_sel) == DmaPeripheral::Uhci0
            {
                if Self::pump_uart_out(&mut c.tx, bus) {
                    c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                    c.tx.pending_coupled = false;
                    c.tx.coupled_desc_ptr = 0;
                    c.tx.coupled_buf_offset = 0;
                }
                let _ = ch_idx; // suppress unused warning in future expansions
            }

            // ── UHCI0 (UART) coupled IN (RX) ─────────────────────────────
            if c.rx.pending_coupled
                && DmaPeripheral::from_sel(c.rx.peri_sel) == DmaPeripheral::Uhci0
            {
                let (eof, in_done) = Self::pump_uart_in(&mut c.rx, bus);
                if in_done {
                    c.rx.int_raw |= IN_DONE_BIT;
                }
                if eof {
                    c.rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
                    c.rx.pending_coupled = false;
                    c.rx.coupled_desc_ptr = 0;
                    c.rx.coupled_buf_offset = 0;
                }
            }

            // ── MEM_TRANS_EN (M2M) path — one-shot, unchanged ────────────
            if !c.pending_m2m {
                continue;
            }
            c.pending_m2m = false;
            c.in_started = false;
            c.out_started = false;

            let out_desc_addr = Self::full_desc_addr(c.tx.link_addr);
            let in_desc_addr = Self::full_desc_addr(c.rx.link_addr);

            // Collect bytes from the OUT (TX) descriptor chain. Owner
            // writeback is gated on this channel's OUT_AUTO_WRBACK.
            let bytes =
                Self::walk_out_chain(bus, out_desc_addr, c.tx.conf0 & OUT_AUTO_WRBACK_BIT != 0);

            if !bytes.is_empty() {
                // Write bytes into the IN (RX) descriptor chain.
                Self::walk_in_chain(bus, in_desc_addr, &bytes);
            }

            // Latch completion flags regardless of byte count (mirrors how
            // real silicon behaves on a zero-length transfer).
            c.rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
            c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
        }
    }
}

impl Peripheral for Esp32s3Gdma {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    /// Level-sensitive IRQ emission: while a channel's INT_ST (RAW & ENA) is
    /// non-zero, re-emit that channel's interrupt-matrix source on every
    /// tick. IN_CHn = base + n; OUT_CHn = base + 5 + n. The source stays
    /// asserted until firmware ACKs via INT_CLR — matching the `systimer`
    /// peripheral's rationale for re-emitting each tick (the bus aggregator
    /// would otherwise race the ISR's own pending-read).
    fn tick(&mut self) -> PeripheralTickResult {
        let mut explicit_irqs = Vec::new();
        for (n, c) in self.channels.iter().enumerate() {
            if c.rx.int_raw & c.rx.int_ena != 0 {
                explicit_irqs.push(self.dma_in_ch0_source + n as u32);
            }
            if c.tx.int_raw & c.tx.int_ena != 0 {
                explicit_irqs.push(self.dma_in_ch0_source + NUM_CHANNELS as u32 + n as u32);
            }
        }

        PeripheralTickResult {
            explicit_irqs: if explicit_irqs.is_empty() {
                None
            } else {
                Some(explicit_irqs)
            },
            ..PeripheralTickResult::default()
        }
    }

    /// True when any channel has a pending MEM_TRANS_EN descriptor walk or a
    /// pending coupled-mode transfer.
    ///
    /// Coupled channels with `pending_coupled` set keep the engine visiting
    /// `tick_with_bus` so the peripheral pumps (UART, SPI2/3, I2S0/1) can
    /// make progress. Cleared per-direction when the transfer completes.
    fn needs_bus_tick(&self) -> bool {
        self.channels
            .iter()
            .any(|c| c.pending_m2m || c.rx.pending_coupled || c.tx.pending_coupled)
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        Esp32s3Gdma::do_tick_with_bus(self, bus);
    }

    /// Walk-free: once the bus attaches a cycle clock under `event-scheduler`,
    /// level IRQs export via [`Self::matrix_irq_sources_into`] and the per-cycle
    /// walk is unnecessary (work settles on MMIO writes / `tick_with_bus`).
    fn uses_scheduler(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.uses_scheduler()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        for (n, c) in self.channels.iter().enumerate() {
            if c.rx.int_raw & c.rx.int_ena != 0 {
                out.push(self.dma_in_ch0_source + n as u32);
            }
            if c.tx.int_raw & c.tx.int_ena != 0 {
                out.push(self.dma_in_ch0_source + NUM_CHANNELS as u32 + n as u32);
            }
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "gdma_tests.rs"]
mod tests;
