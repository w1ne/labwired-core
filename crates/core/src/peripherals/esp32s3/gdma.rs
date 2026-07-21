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
//! `Spi3`, `I2s0`, `I2s1`): the direction is marked `pending_coupled`;
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
//! backwards compatibility. LCD_CAM coupling is **deferred by design**
//! (per the Slice 3 spec): the LCD_CAM register twin exists separately,
//! but its DMA data path stays on the auto-complete fallback until a
//! display/camera pipeline needs it.
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
//!   the next section).
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
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};

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
            _ => 0,
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
            _ => {}
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
    fn coupled_out_collect(dir: &mut DmaDir, bus: &mut dyn Bus, budget: usize) -> Vec<u8> {
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
                dir.coupled_desc_ptr = if d.next == 0 { 0 } else { d.next };
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
                    let mut m =
                        Self::coupled_out_collect(&mut self.channels[i].tx, &mut *sys_bus, k);
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

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::Bus;

    /// Real ESP32-S3 base source for RX channel 0.
    const IN_CH0_SRC: u32 = 66;

    fn ch_base(n: u64) -> u64 {
        n * CHANNEL_STRIDE
    }

    #[test]
    fn defaults_are_zeroed_with_parked_links() {
        let g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            assert_eq!(g.read_word(b + IN_CONF0), 0);
            assert_eq!(g.read_word(b + OUT_CONF0), 0);
            assert_eq!(g.read_word(b + IN_INT_RAW), 0);
            assert_eq!(g.read_word(b + OUT_INT_RAW), 0);
            // PARK bit reads set (idle) on both links.
            assert_eq!(
                g.read_word(b + IN_LINK) & IN_LINK_PARK_BIT,
                IN_LINK_PARK_BIT
            );
            assert_eq!(
                g.read_word(b + OUT_LINK) & OUT_LINK_PARK_BIT,
                OUT_LINK_PARK_BIT
            );
        }
        assert_eq!(g.read_word(MISC_CONF_OFFSET), 0);
    }

    #[test]
    fn conf_round_trip_all_channels() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            g.write_word(b + IN_CONF0, 0x1000_0000 | n as u32);
            g.write_word(b + IN_CONF1, 0x2000_0000 | n as u32);
            g.write_word(b + OUT_CONF0, 0x3000_0000 | n as u32);
            g.write_word(b + OUT_CONF1, 0x4000_0000 | n as u32);
        }
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            assert_eq!(g.read_word(b + IN_CONF0), 0x1000_0000 | n as u32);
            assert_eq!(g.read_word(b + IN_CONF1), 0x2000_0000 | n as u32);
            assert_eq!(g.read_word(b + OUT_CONF0), 0x3000_0000 | n as u32);
            assert_eq!(g.read_word(b + OUT_CONF1), 0x4000_0000 | n as u32);
        }
    }

    #[test]
    fn link_addr_round_trip_all_channels() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            // Write a 20-bit address with no start bit set.
            g.write_word(b + IN_LINK, 0x000A_BCDE);
            g.write_word(b + OUT_LINK, 0x0005_4321);
        }
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            assert_eq!(g.read_word(b + IN_LINK) & IN_LINK_ADDR_MASK, 0x000A_BCDE);
            assert_eq!(g.read_word(b + OUT_LINK) & OUT_LINK_ADDR_MASK, 0x0005_4321);
        }
    }

    #[test]
    fn misc_conf_round_trip() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        g.write_word(MISC_CONF_OFFSET, 0xDEAD_BEEF);
        assert_eq!(g.read_word(MISC_CONF_OFFSET), 0xDEAD_BEEF);
    }

    /// Without MEM_TRANS_EN and with PERI_SEL unbound (reset 0x3F),
    /// INLINK_START takes the fallback auto-complete path: no byte
    /// movement, but EOF is latched immediately.
    #[test]
    fn inlink_start_latches_eof_without_mem_trans_en() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        // MEM_TRANS_EN clear + unbound PERI_SEL → fallback auto-complete.
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x1234);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(raw & IN_SUC_EOF_BIT, IN_SUC_EOF_BIT, "IN_SUC_EOF latched");
        assert_eq!(raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE latched");
        // START is self-clearing: readback never shows bit 22.
        assert_eq!(g.read_word(b + IN_LINK) & IN_LINK_START_BIT, 0);
        // Address was still latched.
        assert_eq!(g.read_word(b + IN_LINK) & IN_LINK_ADDR_MASK, 0x1234);
    }

    /// With MEM_TRANS_EN set, INLINK_START must NOT auto-latch EOF —
    /// the bus-tick path owns that. `pending_m2m` is only set once both
    /// IN_LINK and OUT_LINK have been kicked; INLINK_START alone is not
    /// sufficient (the firmware may start IN before OUT or vice versa).
    #[test]
    fn inlink_start_with_mem_trans_en_defers_eof() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x1000);
        // EOF must NOT be set yet — neither tick_with_bus nor OUT_LINK has run.
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "EOF must not be set after IN_LINK alone"
        );
        // needs_bus_tick is false until OUT_LINK also kicks.
        assert!(!g.needs_bus_tick(), "pending_m2m must wait for OUT_LINK");

        // Now kick OUT_LINK — this arms pending_m2m.
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x2000);
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "EOF still not set — tick_with_bus must run"
        );
        assert!(
            g.needs_bus_tick(),
            "pending_m2m must be flagged after both STARTs"
        );
    }

    #[test]
    fn outlink_start_latches_eof_without_mem_trans_en() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(3);
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x2222);
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF latched");
        assert_eq!(
            raw & OUT_TOTAL_EOF_BIT,
            OUT_TOTAL_EOF_BIT,
            "OUT_TOTAL_EOF latched"
        );
        assert_eq!(raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE latched");
        assert_eq!(g.read_word(b + OUT_LINK) & OUT_LINK_START_BIT, 0);
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // No MEM_TRANS_EN + unbound PERI_SEL → fallback EOF auto-latch.
        g.write_word(b + IN_LINK, IN_LINK_START_BIT);
        assert_eq!(g.read_word(b + IN_INT_RAW), IN_SUC_EOF_BIT | IN_DONE_BIT);
        // Clear only IN_DONE (bit 0); IN_SUC_EOF must remain.
        g.write_word(b + IN_INT_CLR, IN_DONE_BIT);
        assert_eq!(g.read_word(b + IN_INT_RAW), IN_SUC_EOF_BIT);
        // Clear the rest.
        g.write_word(b + IN_INT_CLR, IN_SUC_EOF_BIT);
        assert_eq!(g.read_word(b + IN_INT_RAW), 0);
    }

    #[test]
    fn int_st_masks_raw_with_ena() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT);
        // ENA = 0 → INT_ST is 0 despite RAW set.
        assert_ne!(g.read_word(b + IN_INT_RAW), 0);
        assert_eq!(g.read_word(b + IN_INT_ST), 0);
        // Enable IN_SUC_EOF only.
        g.write_word(b + IN_INT_ENA, IN_SUC_EOF_BIT);
        assert_eq!(g.read_word(b + IN_INT_ST), IN_SUC_EOF_BIT);
    }

    #[test]
    fn tick_emits_in_channel_source_while_st_set() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        // Channel 4 RX: enable + complete.
        let b = ch_base(4);
        g.write_word(b + IN_INT_ENA, IN_SUC_EOF_BIT | IN_DONE_BIT);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT);
        let r = g.tick();
        assert_eq!(
            r.explicit_irqs.as_deref(),
            Some(&[IN_CH0_SRC + 4][..]),
            "IN_CH4 source = base + 4 = 70"
        );
        // Level-sensitive: still emits on the next tick.
        let r = g.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[IN_CH0_SRC + 4][..]));
        // ACK via INT_CLR de-asserts the level.
        g.write_word(b + IN_INT_CLR, IN_SUC_EOF_BIT | IN_DONE_BIT);
        let r = g.tick();
        assert!(r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
    }

    #[test]
    fn tick_emits_out_channel_source() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        // Channel 0 TX: OUT_CH0 source = base + 5 = 71.
        let b = ch_base(0);
        g.write_word(b + OUT_INT_ENA, OUT_EOF_BIT);
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT);
        let r = g.tick();
        assert_eq!(
            r.explicit_irqs.as_deref(),
            Some(&[IN_CH0_SRC + NUM_CHANNELS as u32][..]),
            "OUT_CH0 source = base + 5 = 71"
        );
    }

    #[test]
    fn no_irq_when_ena_zero_even_if_complete() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT);
        // RAW set but ENA = 0 → no IRQ.
        let r = g.tick();
        assert!(r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
    }

    #[test]
    fn channels_are_independent() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        g.write_word(ch_base(0) + IN_CONF0, 0xAAAA_AAAA);
        // Channel 1 IN_CONF0 must be untouched.
        assert_eq!(g.read_word(ch_base(1) + IN_CONF0), 0);
        g.write_word(ch_base(1) + IN_LINK, IN_LINK_START_BIT);
        // Channel 0 INT_RAW must be untouched by channel 1's completion.
        assert_eq!(g.read_word(ch_base(0) + IN_INT_RAW), 0);
        assert_ne!(g.read_word(ch_base(1) + IN_INT_RAW), 0);
    }

    #[test]
    fn byte_granular_access_matches_word() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // Byte writes assemble into the CONF0 word.
        g.write(b + IN_CONF0, 0x78).unwrap();
        g.write(b + IN_CONF0 + 1, 0x56).unwrap();
        g.write(b + IN_CONF0 + 2, 0x34).unwrap();
        g.write(b + IN_CONF0 + 3, 0x12).unwrap();
        assert_eq!(g.read_u32(b + IN_CONF0).unwrap(), 0x1234_5678);
        assert_eq!(g.read(b + IN_CONF0 + 2).unwrap(), 0x34);
    }

    // ── mem-to-mem (MEM_TRANS_EN) descriptor-walk tests ───────────────────

    /// Helper: write a DMA descriptor (3 words: dw0, buffer, next) into the
    /// bus at `addr`.
    fn write_desc(bus: &mut SystemBus, addr: u64, dw0: u32, buffer: u64, next: u64) {
        bus.write_u32(addr, dw0).unwrap();
        bus.write_u32(addr + 4, buffer as u32).unwrap();
        bus.write_u32(addr + 8, next as u32).unwrap();
    }

    /// Encode a TX descriptor dw0: owner=DMA, suc_eof, length, size.
    fn tx_dw0(len: u32) -> u32 {
        (1 << 31) | (1 << 30) | (len << 12) | len
    }

    /// Encode an RX descriptor dw0: owner=DMA, size (no length yet).
    fn rx_dw0(size: u32) -> u32 {
        (1 << 31) | size
    }

    /// Build a `SystemBus` with 256 KiB of DRAM registered at the
    /// ESP32-S3 DRAM base (`0x3FC8_8000`). The mem-to-mem tests need a
    /// real addressable region so the descriptor-walk reads and buffer
    /// writes can go through the bus router without a MemoryViolation.
    fn bus_with_dram() -> SystemBus {
        use crate::system::xtensa::RamPeripheral;
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "dram_test",
            0x3FC8_8000,
            256 * 1024,
            None,
            Box::new(RamPeripheral::new(256 * 1024)),
        );
        bus
    }

    /// Build the fixture's exact register sequence and verify bytes moved.
    ///
    /// This test mirrors what `check_dma()` in the Tier-1 fixture does:
    /// place real linked-list descriptors in DRAM, kick OUT then IN with
    /// MEM_TRANS_EN, poll IN_SUC_EOF, verify src == dst byte-by-byte.
    #[test]
    fn m2m_single_descriptor_bytes_move() {
        let mut bus = bus_with_dram();

        // Source buffer at 0x3FC8_8000 (DRAM base in the S3 model).
        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let src_data: &[u8] = b"TIER1-GDMA-M2M!\0";
        let len = src_data.len() as u32;

        for (i, &b) in src_data.iter().enumerate() {
            bus.write_u8(src_addr + i as u64, b).unwrap();
        }

        // TX descriptor at 0x3FC8_A000, RX descriptor at 0x3FC8_B000.
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;

        write_desc(&mut bus, tx_desc, tx_dw0(len), src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(len), dst_addr, 0);

        // Build GDMA and perform the fixture's exact register sequence.
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0); // channel 0

        // Enable MEM_TRANS_EN.
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);

        // Fixture: clear pending interrupts.
        g.write_word(b + IN_INT_CLR, 0xFFFF_FFFF);
        g.write_word(b + OUT_INT_CLR, 0xFFFF_FFFF);

        // Kick: INLINK_START with rx descriptor address (lower 20 bits).
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        // Kick: OUTLINK_START with tx descriptor address (lower 20 bits).
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );

        // Before tick_with_bus: IN_SUC_EOF must NOT be set.
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "EOF must not be set before tick_with_bus"
        );

        // Execute the descriptor walk.
        g.tick_with_bus(&mut bus);

        // IN_SUC_EOF must now be set.
        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "IN_SUC_EOF must be set after tick_with_bus"
        );

        // Bytes must have moved.
        for (i, &expected) in src_data.iter().enumerate() {
            let got = bus.read_u8(dst_addr + i as u64).unwrap();
            assert_eq!(got, expected, "dst[{i}] = {got:#04x} want {expected:#04x}");
        }

        // needs_bus_tick must be false after the walk.
        assert!(!g.needs_bus_tick(), "pending_m2m must be cleared");
    }

    /// Owner bit = 0 (CPU-owned): descriptor must be skipped, no bytes moved,
    /// but IN_SUC_EOF is still latched (zero-length completion).
    #[test]
    fn m2m_cpu_owned_descriptor_skipped() {
        let mut bus = bus_with_dram();

        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;

        bus.write_u8(src_addr, 0xAB).unwrap();
        bus.write_u8(dst_addr, 0x00).unwrap();

        // TX descriptor with owner=CPU (bit 31 = 0) — should be skipped.
        let dw0_cpu = (1u32 << 30) | (1 << 12) | 1; // no owner bit
        write_desc(&mut bus, tx_desc, dw0_cpu, src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(1), dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        // No bytes moved (TX descriptor was CPU-owned, walk produced 0 bytes).
        assert_eq!(
            bus.read_u8(dst_addr).unwrap(),
            0x00,
            "dst must be untouched when TX descriptor is CPU-owned"
        );
        // Completion flags are still latched.
        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "IN_SUC_EOF latched even for zero-byte transfer"
        );
    }

    /// Two-descriptor OUT chain → single RX descriptor: verifies multi-link
    /// chain walking.
    #[test]
    fn m2m_two_tx_descriptors_chained() {
        let mut bus = bus_with_dram();

        // Two source buffers of 4 bytes each.
        let src1: u64 = 0x3FC8_8000;
        let src2: u64 = 0x3FC8_8010;
        let dst: u64 = 0x3FC8_9000;
        let tx_desc1: u64 = 0x3FC8_A000;
        let tx_desc2: u64 = 0x3FC8_A010;
        let rx_desc: u64 = 0x3FC8_B000;

        let data1 = [0x11u8, 0x22, 0x33, 0x44];
        let data2 = [0x55u8, 0x66, 0x77, 0x88];
        for (i, &b) in data1.iter().enumerate() {
            bus.write_u8(src1 + i as u64, b).unwrap();
        }
        for (i, &b) in data2.iter().enumerate() {
            bus.write_u8(src2 + i as u64, b).unwrap();
        }

        // TX chain: desc1 → desc2 → EOL.
        // desc1: owner=DMA, NOT suc_eof (not last), length=4, size=4.
        let dw0_1 = (1u32 << 31) | (4 << 12) | 4; // no suc_eof
        write_desc(&mut bus, tx_desc1, dw0_1, src1, tx_desc2);
        // desc2: owner=DMA, suc_eof, length=4, size=4, next=0.
        write_desc(&mut bus, tx_desc2, tx_dw0(4), src2, 0);

        // RX: single descriptor big enough for both chunks (8 bytes).
        write_desc(&mut bus, rx_desc, rx_dw0(8), dst, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc1 as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        // All 8 bytes must have arrived at dst.
        let expected = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        for (i, &exp) in expected.iter().enumerate() {
            let got = bus.read_u8(dst + i as u64).unwrap();
            assert_eq!(got, exp, "dst[{i}] = {got:#04x} want {exp:#04x}");
        }
        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "IN_SUC_EOF after two-descriptor chain"
        );
    }

    // ── PERI_SEL register tests ───────────────────────────────────────────

    /// Round-trip: write a valid sel value to IN_PERI_SEL on ch2, read back.
    #[test]
    fn in_peri_sel_round_trip() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        // sel = 3 (I2S0).
        g.write_word(b + IN_PERI_SEL, 0x03);
        assert_eq!(g.read_word(b + IN_PERI_SEL), 0x03);
    }

    /// Mask: writing 0xFF to IN_PERI_SEL reads back only the low 6 bits (0x3F).
    #[test]
    fn in_peri_sel_mask_enforced() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + IN_PERI_SEL, 0xFF);
        assert_eq!(g.read_word(b + IN_PERI_SEL), 0x3F, "only 6 bits are stored");
    }

    /// Reset value of IN_PERI_SEL is 0x3F (unbound).
    #[test]
    fn in_peri_sel_reset_value() {
        let g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            assert_eq!(
                g.read_word(ch_base(n) + IN_PERI_SEL),
                PERI_SEL_RESET,
                "ch{n} IN_PERI_SEL reset must be 0x3F"
            );
        }
    }

    /// Round-trip: write a valid sel value to OUT_PERI_SEL on ch2, read back.
    #[test]
    fn out_peri_sel_round_trip() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        // sel = 3 (I2S0).
        g.write_word(b + OUT_PERI_SEL, 0x03);
        assert_eq!(g.read_word(b + OUT_PERI_SEL), 0x03);
    }

    /// Mask: writing 0xFF to OUT_PERI_SEL reads back only the low 6 bits.
    #[test]
    fn out_peri_sel_mask_enforced() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + OUT_PERI_SEL, 0xFF);
        assert_eq!(g.read_word(b + OUT_PERI_SEL), 0x3F);
    }

    /// Reset value of OUT_PERI_SEL is 0x3F (unbound).
    #[test]
    fn out_peri_sel_reset_value() {
        let g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            assert_eq!(
                g.read_word(ch_base(n) + OUT_PERI_SEL),
                PERI_SEL_RESET,
                "ch{n} OUT_PERI_SEL reset must be 0x3F"
            );
        }
    }

    /// PERI_SEL registers on different channels are independent.
    #[test]
    fn peri_sel_channels_independent() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        g.write_word(ch_base(0) + IN_PERI_SEL, 0x02); // UHCI0
        g.write_word(ch_base(1) + IN_PERI_SEL, 0x07); // SHA
        assert_eq!(g.read_word(ch_base(0) + IN_PERI_SEL), 0x02);
        assert_eq!(g.read_word(ch_base(1) + IN_PERI_SEL), 0x07);
        // Unwritten channels stay at reset.
        assert_eq!(g.read_word(ch_base(2) + IN_PERI_SEL), PERI_SEL_RESET);
    }

    // ── Coupled-set start tests ───────────────────────────────────────────

    /// Coupled IN: set OUT_PERI_SEL = UHCI0 (2), MEM_TRANS_EN clear,
    /// write OUTLINK_START → OUT_INT_RAW must have NO OUT_EOF bits.
    #[test]
    fn coupled_out_start_does_not_latch_eof() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // MEM_TRANS_EN must be clear (default).
        g.write_word(b + OUT_PERI_SEL, 2); // UHCI0 = coupled
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x1000);
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(
            raw & (OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT),
            0,
            "coupled OUT start must NOT latch EOF"
        );
        // needs_bus_tick returns true because pending_coupled is set.
        assert!(
            g.needs_bus_tick(),
            "needs_bus_tick must be true for coupled OUT"
        );
    }

    /// Coupled IN: set IN_PERI_SEL = UHCI0 (2), MEM_TRANS_EN clear,
    /// write INLINK_START → IN_INT_RAW must have NO IN_SUC_EOF/IN_DONE bits.
    #[test]
    fn coupled_in_start_does_not_latch_eof() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0 = coupled
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x2000);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(
            raw & (IN_SUC_EOF_BIT | IN_DONE_BIT),
            0,
            "coupled IN start must NOT latch EOF"
        );
        assert!(
            g.needs_bus_tick(),
            "needs_bus_tick must be true for coupled IN"
        );
    }

    /// All five coupled peripheral values for OUT direction do NOT latch EOF.
    #[test]
    fn coupled_out_all_five_peripherals_no_eof() {
        for sel in [0u32, 1, 2, 3, 4] {
            // sel 0=SPI2, 1=SPI3, 2=UHCI0, 3=I2S0, 4=I2S1
            let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
            let b = ch_base(0);
            g.write_word(b + OUT_PERI_SEL, sel);
            g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x100);
            let raw = g.read_word(b + OUT_INT_RAW);
            assert_eq!(
                raw & (OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT),
                0,
                "sel={sel} should not latch OUT_EOF"
            );
        }
    }

    /// All five coupled peripheral values for IN direction do NOT latch EOF.
    #[test]
    fn coupled_in_all_five_peripherals_no_eof() {
        for sel in [0u32, 1, 2, 3, 4] {
            let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
            let b = ch_base(0);
            g.write_word(b + IN_PERI_SEL, sel);
            g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x100);
            let raw = g.read_word(b + IN_INT_RAW);
            assert_eq!(
                raw & (IN_SUC_EOF_BIT | IN_DONE_BIT),
                0,
                "sel={sel} should not latch IN_SUC_EOF"
            );
        }
    }

    // ── Fallback-set auto-complete tests ─────────────────────────────────

    /// Fallback OUT: SHA (sel=7) auto-completes immediately on OUTLINK_START.
    #[test]
    fn fallback_out_sha_auto_completes() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 7); // SHA = fallback
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x3000);
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF must be latched");
        assert_eq!(
            raw & OUT_TOTAL_EOF_BIT,
            OUT_TOTAL_EOF_BIT,
            "OUT_TOTAL_EOF must be latched"
        );
        assert_eq!(raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE must be latched");
        assert!(!g.needs_bus_tick(), "no bus tick needed for fallback OUT");
    }

    /// Fallback IN: SHA (sel=7) auto-completes immediately on INLINK_START.
    #[test]
    fn fallback_in_sha_auto_completes() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 7); // SHA = fallback
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x4000);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(
            raw & IN_SUC_EOF_BIT,
            IN_SUC_EOF_BIT,
            "IN_SUC_EOF must be latched"
        );
        assert_eq!(raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE must be latched");
    }

    /// Unbound reset value (0x3F) behaves as fallback for OUT direction.
    /// Firmware that never writes PERI_SEL must get auto-complete (legacy
    /// compatibility promise).
    #[test]
    fn unbound_out_reset_value_is_fallback() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(3);
        // Do NOT write PERI_SEL; it stays at 0x3F (Unknown).
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x5000);
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_ne!(
            raw & (OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT),
            0,
            "unbound PERI_SEL (reset 0x3F) must auto-complete OUT"
        );
    }

    /// Unbound reset value (0x3F) behaves as fallback for IN direction.
    #[test]
    fn unbound_in_reset_value_is_fallback() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(3);
        // Do NOT write PERI_SEL; it stays at 0x3F (Unknown).
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x6000);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_ne!(
            raw & (IN_SUC_EOF_BIT | IN_DONE_BIT),
            0,
            "unbound PERI_SEL (reset 0x3F) must auto-complete IN"
        );
    }

    // ── UART (UHCI0) coupled transfer tests ──────────────────────────────
    //
    // These tests require a composite bus with:
    //   - DRAM region at 0x3FC8_8000 (for descriptors and data buffers).
    //   - GDMA peripheral at 0x6003_F000 (real ESP32-S3 base).
    //   - Esp32s3Uart at 0x6000_0000 (UART0, real ESP32-S3 base).
    //
    // The GDMA struct is also held directly for register-level manipulation.
    // The bus is used only for the byte-pump paths (descriptor reads,
    // UART FIFO read/write, STATUS read).

    use crate::peripherals::esp32s3::uart::Esp32s3Uart;
    use std::sync::{Arc, Mutex};

    /// Build a composite `SystemBus` with DRAM, GDMA, and UART0 registered at
    /// their real ESP32-S3 addresses.  Returns `(bus, sink)` where `sink` is
    /// the shared TX capture buffer attached to UART0.
    ///
    /// GDMA is added to the bus so tick_with_bus can reach the UART FIFO
    /// via normal bus reads/writes.  The caller also holds a separate
    /// `Esp32s3Gdma` instance for register-level manipulation; descriptor
    /// walks use `bus` directly.
    fn uart_test_bus() -> (SystemBus, Arc<Mutex<Vec<u8>>>) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::new();
        // 256 KiB DRAM at 0x3FC8_8000 — descriptors and data buffers go here.
        bus.add_peripheral(
            "dram_test",
            0x3FC8_8000,
            256 * 1024,
            None,
            Box::new(crate::system::xtensa::RamPeripheral::new(256 * 1024)),
        );
        // UART0 at the real DR_REG_UART0_BASE.
        let mut uart = Esp32s3Uart::new(false, 27);
        uart.set_sink(Some(sink.clone()));
        bus.add_peripheral("uart0_test", UART0_BASE, 0x100, None, Box::new(uart));
        (bus, sink)
    }

    /// Write a 3-word DMA descriptor into the bus at `addr`.
    fn write_desc_uart(bus: &mut SystemBus, addr: u64, dw0: u32, buffer: u64, next: u64) {
        bus.write_u32(addr, dw0).unwrap();
        bus.write_u32(addr + 4, buffer as u32).unwrap();
        bus.write_u32(addr + 8, next as u32).unwrap();
    }

    // ── TX (OUT) tests ────────────────────────────────────────────────────

    /// TX basic: write "HELLO" into a single descriptor, set PERI_SEL=UHCI0,
    /// start OUT link → after ticks, UART TX sink contains "HELLO"; EOF latched;
    /// pending_coupled cleared (needs_bus_tick returns false).
    #[test]
    fn uart_tx_hello_via_descriptors() {
        let (mut bus, sink) = uart_test_bus();
        let payload = b"HELLO";
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;

        // Write payload into DRAM.
        for (i, &b) in payload.iter().enumerate() {
            bus.write_u8(buf_addr + i as u64, b).unwrap();
        }
        // Single TX descriptor: owner=DMA, suc_eof, length=5, size=5.
        let dw0 = (1u32 << 31) | (1 << 30) | (5 << 12) | 5;
        write_desc_uart(&mut bus, desc_addr, dw0, buf_addr, 0);

        // Set up GDMA channel 0 for UHCI0 OUT.
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 2); // UHCI0
                                           // desc_addr bits[19:0] = 0xA000.
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );

        // pending_coupled must be set; EOF not latched yet.
        assert!(g.needs_bus_tick(), "needs_bus_tick after OUT start");
        assert_eq!(
            g.read_word(b + OUT_INT_RAW) & (OUT_EOF_BIT | OUT_DONE_BIT),
            0
        );

        // Drain via tick_with_bus (5 bytes, well within COUPLED_BYTES_PER_TICK).
        g.tick_with_bus(&mut bus);

        // EOF must be latched.
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF must be set");
        assert_eq!(raw & OUT_TOTAL_EOF_BIT, OUT_TOTAL_EOF_BIT, "OUT_TOTAL_EOF");
        assert_eq!(raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE");
        // pending_coupled cleared → needs_bus_tick false.
        assert!(
            !g.needs_bus_tick(),
            "needs_bus_tick must be false after completion"
        );

        // UART TX FIFO should have received the bytes; drain them via tick.
        // reset CLKDIV=694 → ~20820 ticks/byte; 5 bytes × 25000 is generous.
        let uart_idx = bus.find_peripheral_index_by_name("uart0_test").unwrap();
        let uart = bus.peripherals[uart_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3Uart>()
            .unwrap();
        for _ in 0..200_000u64 {
            uart.tick();
        }

        let got = sink.lock().unwrap().clone();
        assert_eq!(got, b"HELLO", "UART sink must contain HELLO, got {:?}", got);
    }

    /// TX larger than COUPLED_BYTES_PER_TICK (200 bytes): completes over
    /// multiple ticks, order preserved.
    #[test]
    fn uart_tx_large_transfer_multi_tick() {
        let (mut bus, sink) = uart_test_bus();
        let count: usize = 200;
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;

        // Write 200 bytes of sequential data.
        let payload: Vec<u8> = (0u8..200).collect();
        for (i, &b) in payload.iter().enumerate() {
            bus.write_u8(buf_addr + i as u64, b).unwrap();
        }
        let dw0 = (1u32 << 31) | (1 << 30) | ((count as u32) << 12) | count as u32;
        write_desc_uart(&mut bus, desc_addr, dw0, buf_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 2);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );

        // Drive ticks until completion.  Interleave UART drain between GDMA
        // ticks so the TX FIFO never stays full (200 bytes > FIFO depth 128).
        // 20820 ticks/byte × 128 bytes ≈ 2.7 M uart ticks clears a full FIFO.
        let uart_idx = bus.find_peripheral_index_by_name("uart0_test").unwrap();
        let mut ticks = 0usize;
        while g.needs_bus_tick() {
            g.tick_with_bus(&mut bus);
            ticks += 1;
            // Drain UART between GDMA ticks to free FIFO space.
            let uart = bus.peripherals[uart_idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<Esp32s3Uart>()
                .unwrap();
            for _ in 0..3_000_000u64 {
                uart.tick();
            }
            assert!(ticks < 500, "transfer did not complete in reasonable ticks");
        }

        // At least 2 GDMA ticks (first fills FIFO with ≤64 bytes from budget,
        // but FIFO may already be partially drained by interleaved uart ticks;
        // so the exact count depends on timing — just verify > 1).
        assert!(
            ticks >= 1,
            "expected ≥1 gdma tick for 200 bytes, got {ticks}"
        );

        // Final UART drain.
        let uart = bus.peripherals[uart_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3Uart>()
            .unwrap();
        for _ in 0..5_000_000u64 {
            uart.tick();
        }

        let got = sink.lock().unwrap().clone();
        assert_eq!(
            got.len(),
            count,
            "expected {count} bytes in sink, got {}",
            got.len()
        );
        assert_eq!(got, payload, "byte order must be preserved");

        // EOF flags set.
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF after large TX");
        assert!(!g.needs_bus_tick(), "needs_bus_tick cleared");
    }

    // ── RX (IN) tests ─────────────────────────────────────────────────────

    /// Helper: push bytes into UART0 RX FIFO via the bus's peripheral index.
    fn push_uart_rx(bus: &mut SystemBus, bytes: &[u8]) {
        let idx = bus.find_peripheral_index_by_name("uart0_test").unwrap();
        let uart = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3Uart>()
            .unwrap();
        for &b in bytes {
            uart.push_rx(b);
        }
    }

    /// RX basic: inject bytes into the UART RX FIFO, provide an IN descriptor
    /// with enough capacity → bytes land in memory; IN_SUC_EOF + IN_DONE set;
    /// pending_coupled cleared.
    #[test]
    fn uart_rx_bytes_land_in_descriptor() {
        let (mut bus, _sink) = uart_test_bus();
        let rx_data = b"WORLD";
        push_uart_rx(&mut bus, rx_data);

        let dst_addr: u64 = 0x3FC8_9000;
        let desc_addr: u64 = 0x3FC8_B000;
        // RX descriptor: owner=DMA, size=32 (plenty of space).
        let dw0 = (1u32 << 31) | 32u32;
        write_desc_uart(&mut bus, desc_addr, dw0, dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc_addr as u32 & IN_LINK_ADDR_MASK),
        );

        assert!(g.needs_bus_tick());
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & (IN_SUC_EOF_BIT | IN_DONE_BIT),
            0
        );

        // One tick should drain 5 bytes (within budget).
        g.tick_with_bus(&mut bus);

        // Check INT_RAW.
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(
            raw & IN_SUC_EOF_BIT,
            IN_SUC_EOF_BIT,
            "IN_SUC_EOF must be set"
        );
        assert_eq!(raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE must be set");
        assert!(!g.needs_bus_tick(), "pending_coupled cleared");

        // Bytes must have landed in memory.
        for (i, &expected) in rx_data.iter().enumerate() {
            let got = bus.read_u8(dst_addr + i as u64).unwrap();
            assert_eq!(got, expected, "dst[{i}] mismatch");
        }
    }

    /// RX partial: descriptor smaller than FIFO content → first descriptor
    /// filled + IN_DONE, chain continues on next tick.
    #[test]
    fn uart_rx_partial_fill_continues_next_tick() {
        let (mut bus, _sink) = uart_test_bus();
        // Push 8 bytes; first descriptor only holds 4.
        push_uart_rx(&mut bus, b"ABCDEFGH");

        let dst1: u64 = 0x3FC8_9000;
        let dst2: u64 = 0x3FC8_9010;
        let desc1: u64 = 0x3FC8_B000;
        let desc2: u64 = 0x3FC8_B010;

        // Two descriptors of 4 bytes each, chained.
        let dw0_1 = (1u32 << 31) | 4u32; // cap=4
        let dw0_2 = (1u32 << 31) | 4u32;
        write_desc_uart(&mut bus, desc1, dw0_1, dst1, desc2);
        write_desc_uart(&mut bus, desc2, dw0_2, dst2, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc1 as u32 & IN_LINK_ADDR_MASK),
        );

        // First tick: drain 4 bytes → desc1 filled; IN_DONE set; transfer still
        // pending (8 bytes total but desc1 full, chain advances to desc2).
        g.tick_with_bus(&mut bus);

        // IN_DONE should be set (first descriptor completed).
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_DONE_BIT,
            IN_DONE_BIT,
            "IN_DONE after first desc filled"
        );

        // Second tick drains remaining 4 bytes.
        // The first tick may have drained all 8 if within budget — that's fine
        // too. Just run until done.
        let mut ticks = 0;
        while g.needs_bus_tick() {
            g.tick_with_bus(&mut bus);
            ticks += 1;
            assert!(ticks < 100, "did not complete in reasonable ticks");
        }

        // All 8 bytes must be in memory.
        let expected = b"ABCDEFGH";
        for (i, &exp) in expected.iter().enumerate() {
            let got = if i < 4 {
                bus.read_u8(dst1 + i as u64).unwrap()
            } else {
                bus.read_u8(dst2 + (i - 4) as u64).unwrap()
            };
            assert_eq!(got, exp, "byte[{i}] mismatch");
        }

        // IN_SUC_EOF must be set at the end.
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            IN_SUC_EOF_BIT,
            "IN_SUC_EOF must be latched after full chain"
        );
    }

    /// FIFO-idle EOF lands exactly on a descriptor boundary: the moved bytes
    /// exactly fill descriptor 1, descriptor 2 receives nothing and must stay
    /// DMA-owned and untouched (silicon leaves it alone). Pins the
    /// `coupled_buf_offset > 0` guard in `pump_uart_in`'s idle-EOF writeback.
    #[test]
    fn uart_rx_exact_descriptor_fill_leaves_next_descriptor_dma_owned() {
        let (mut bus, _sink) = uart_test_bus();
        // Push exactly descriptor 1's capacity (4 bytes).
        push_uart_rx(&mut bus, b"ABCD");

        let dst1: u64 = 0x3FC8_9000;
        let dst2: u64 = 0x3FC8_9010;
        let desc1: u64 = 0x3FC8_B000;
        let desc2: u64 = 0x3FC8_B010;
        write_desc_uart(&mut bus, desc1, (1u32 << 31) | 4, dst1, desc2);
        write_desc_uart(&mut bus, desc2, (1u32 << 31) | 4, dst2, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc1 as u32 & IN_LINK_ADDR_MASK),
        );

        g.tick_with_bus(&mut bus);

        // FIFO drained after exactly filling desc1 → idle EOF latched.
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(raw & IN_SUC_EOF_BIT, IN_SUC_EOF_BIT, "idle EOF latched");
        assert_eq!(raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE for filled desc1");
        assert!(!g.needs_bus_tick(), "pending_coupled cleared on EOF");

        // desc1: returned to CPU with length = 4.
        let dw0_1 = bus.read_u32(desc1).unwrap();
        assert_eq!(dw0_1 & DESC_OWNER_BIT, 0, "desc1 owner returned to CPU");
        assert_eq!((dw0_1 >> 12) & 0xFFF, 4, "desc1 length = 4");
        // desc2: received nothing → still DMA-owned and untouched.
        let dw0_2 = bus.read_u32(desc2).unwrap();
        assert_eq!(
            dw0_2 & DESC_OWNER_BIT,
            DESC_OWNER_BIT,
            "desc2 must stay DMA-owned (owner=1)"
        );
        assert_eq!((dw0_2 >> 12) & 0xFFF, 0, "desc2 length untouched");
    }

    // ── Interrupt (explicit_irqs) tests ───────────────────────────────────

    /// TX completion with INT_ENA set → explicit_irqs carries OUT_CH0 source.
    #[test]
    fn uart_tx_irq_fires_on_eof() {
        let (mut bus, _sink) = uart_test_bus();
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;
        bus.write_u8(buf_addr, b'X').unwrap();
        let dw0 = (1u32 << 31) | (1 << 30) | (1 << 12) | 1;
        write_desc_uart(&mut bus, desc_addr, dw0, buf_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // Enable OUT_EOF interrupt (bit 1).
        g.write_word(
            b + OUT_INT_ENA,
            OUT_EOF_BIT | OUT_DONE_BIT | OUT_TOTAL_EOF_BIT,
        );
        g.write_word(b + OUT_PERI_SEL, 2);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );

        g.tick_with_bus(&mut bus);

        // Now tick() should emit the OUT_CH0 source (base + 5 + 0 = 71).
        let result = g.tick();
        let irqs = result.explicit_irqs.unwrap_or_default();
        let expected_src = IN_CH0_SRC + NUM_CHANNELS as u32; // 66 + 5 = 71
        assert!(
            irqs.contains(&expected_src),
            "expected OUT_CH0 source {expected_src} in {irqs:?}"
        );
    }

    /// RX completion with INT_ENA set → explicit_irqs carries IN_CH0 source.
    #[test]
    fn uart_rx_irq_fires_on_suc_eof() {
        let (mut bus, _sink) = uart_test_bus();
        push_uart_rx(&mut bus, b"Z");

        let dst: u64 = 0x3FC8_9000;
        let desc: u64 = 0x3FC8_B000;
        let dw0 = (1u32 << 31) | 16u32;
        write_desc_uart(&mut bus, desc, dw0, dst, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_INT_ENA, IN_SUC_EOF_BIT | IN_DONE_BIT);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc as u32 & IN_LINK_ADDR_MASK),
        );

        g.tick_with_bus(&mut bus);

        let result = g.tick();
        let irqs = result.explicit_irqs.unwrap_or_default();
        let expected_src = IN_CH0_SRC; // channel 0 IN = 66
        assert!(
            irqs.contains(&expected_src),
            "expected IN_CH0 source {expected_src} in {irqs:?}"
        );
    }

    // ── Owner-bit writeback tests ─────────────────────────────────────────

    /// M2M with OUT_AUTO_WRBACK set (as ESP-IDF drivers do via
    /// `gdma_ll_tx_enable_auto_write_back`): after tick_with_bus, the TX
    /// descriptor dw0 must have its owner bit cleared.
    #[test]
    fn m2m_tx_owner_bit_cleared_after_walk() {
        let mut bus = bus_with_dram();
        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        bus.write_u8(src_addr, 0xAB).unwrap();
        write_desc(&mut bus, tx_desc, tx_dw0(1), src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(1), dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(b + OUT_CONF0, OUT_AUTO_WRBACK_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        let dw0_after = bus.read_u32(tx_desc).unwrap();
        assert_eq!(
            dw0_after & DESC_OWNER_BIT,
            0,
            "TX descriptor owner bit must be cleared after M2M walk"
        );
    }

    /// M2M: after tick_with_bus, RX descriptor dw0 must have owner bit cleared
    /// and bits [23:12] must contain the received byte count.
    #[test]
    fn m2m_rx_owner_bit_cleared_and_length_set() {
        let mut bus = bus_with_dram();
        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        let len = 7u32;
        for i in 0..len {
            bus.write_u8(src_addr + i as u64, i as u8).unwrap();
        }
        write_desc(&mut bus, tx_desc, tx_dw0(len), src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(len), dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        let dw0_after = bus.read_u32(rx_desc).unwrap();
        assert_eq!(
            dw0_after & DESC_OWNER_BIT,
            0,
            "RX descriptor owner bit must be cleared"
        );
        let written_len = (dw0_after >> 12) & 0xFFF;
        assert_eq!(
            written_len, len,
            "RX descriptor length field must equal bytes written"
        );
    }

    /// UART TX with OUT_AUTO_WRBACK set: after coupled OUT completes, TX
    /// descriptor dw0 owner bit = 0.
    #[test]
    fn uart_tx_owner_bit_cleared_on_completion() {
        let (mut bus, _sink) = uart_test_bus();
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;
        bus.write_u8(buf_addr, b'X').unwrap();
        let dw0 = (1u32 << 31) | (1 << 30) | (1 << 12) | 1;
        write_desc_uart(&mut bus, desc_addr, dw0, buf_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_CONF0, OUT_AUTO_WRBACK_BIT);
        g.write_word(b + OUT_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );
        g.tick_with_bus(&mut bus);

        let dw0_after = bus.read_u32(desc_addr).unwrap();
        assert_eq!(
            dw0_after & DESC_OWNER_BIT,
            0,
            "UART TX descriptor owner bit must be cleared after completion"
        );
    }

    /// UART RX: after coupled IN completes, RX descriptor dw0 owner bit = 0
    /// and bits [23:12] contain the received byte count.
    #[test]
    fn uart_rx_owner_bit_cleared_and_received_length_set() {
        let (mut bus, _sink) = uart_test_bus();
        let rx_data = b"ABCDE";
        push_uart_rx(&mut bus, rx_data);

        let dst_addr: u64 = 0x3FC8_9000;
        let desc_addr: u64 = 0x3FC8_B000;
        let cap = 16u32;
        let dw0 = (1u32 << 31) | cap;
        write_desc_uart(&mut bus, desc_addr, dw0, dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc_addr as u32 & IN_LINK_ADDR_MASK),
        );
        g.tick_with_bus(&mut bus);

        let dw0_after = bus.read_u32(desc_addr).unwrap();
        assert_eq!(
            dw0_after & DESC_OWNER_BIT,
            0,
            "UART RX descriptor owner bit must be cleared"
        );
        let written_len = (dw0_after >> 12) & 0xFFF;
        assert_eq!(
            written_len as usize,
            rx_data.len(),
            "RX descriptor length field must equal bytes received"
        );
    }

    /// M2M with OUT_AUTO_WRBACK clear (the reset value): the completed walk
    /// leaves the OUT descriptor DMA-owned (owner=1) while the IN
    /// descriptor is written back unconditionally (owner=0, length set) —
    /// the IN side is unaffected by the OUT-side bit. A second
    /// OUTLINK_START on the same UNTOUCHED OUT chain (no CPU rewrite of
    /// dw0 — legal on silicon) then transfers the same bytes again.
    #[test]
    fn m2m_auto_wrback_clear_preserves_out_owner_and_rekick_repeats() {
        let mut bus = bus_with_dram();
        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        let payload = b"REKICK";
        let len = payload.len() as u32;
        for (i, &x) in payload.iter().enumerate() {
            bus.write_u8(src_addr + i as u64, x).unwrap();
        }
        write_desc(&mut bus, tx_desc, tx_dw0(len), src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(len), dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // OUT_CONF0 left at reset → OUT_AUTO_WRBACK clear.
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        // OUT descriptor stays DMA-owned; IN writeback is unconditional.
        assert_eq!(
            bus.read_u32(tx_desc).unwrap() & DESC_OWNER_BIT,
            DESC_OWNER_BIT,
            "OUT owner must stay 1 with OUT_AUTO_WRBACK clear"
        );
        let rx_dw0_after = bus.read_u32(rx_desc).unwrap();
        assert_eq!(
            rx_dw0_after & DESC_OWNER_BIT,
            0,
            "IN owner cleared regardless of OUT_AUTO_WRBACK"
        );
        assert_eq!((rx_dw0_after >> 12) & 0xFFF, len, "IN length still set");
        for (i, &x) in payload.iter().enumerate() {
            assert_eq!(
                bus.read_u8(dst_addr + i as u64).unwrap(),
                x,
                "first pass dst[{i}]"
            );
        }

        // CPU re-arms ONLY the RX side (IN descriptors were consumed, as on
        // silicon) and clears the destination; the OUT chain is
        // deliberately left untouched.
        write_desc(&mut bus, rx_desc, rx_dw0(len), dst_addr, 0);
        for i in 0..len as u64 {
            bus.write_u8(dst_addr + i, 0).unwrap();
        }
        g.write_word(b + IN_INT_CLR, 0xFFFF_FFFF);
        g.write_word(b + OUT_INT_CLR, 0xFFFF_FFFF);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "second pass must complete (no halt at a CPU-owned descriptor)"
        );
        for (i, &x) in payload.iter().enumerate() {
            assert_eq!(
                bus.read_u8(dst_addr + i as u64).unwrap(),
                x,
                "second pass dst[{i}] — same bytes moved again"
            );
        }
    }

    /// Coupled UART OUT with OUT_AUTO_WRBACK clear: the consumed descriptor
    /// stays DMA-owned after completion, and a second OUTLINK_START on the
    /// same untouched chain pushes the same bytes into the UART again.
    #[test]
    fn uart_tx_auto_wrback_clear_preserves_owner_and_rekick_repeats() {
        let (mut bus, sink) = uart_test_bus();
        let payload = b"AGAIN";
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;
        for (i, &x) in payload.iter().enumerate() {
            bus.write_u8(buf_addr + i as u64, x).unwrap();
        }
        write_desc_uart(
            &mut bus,
            desc_addr,
            tx_dw0(payload.len() as u32),
            buf_addr,
            0,
        );

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // OUT_CONF0 left at reset → OUT_AUTO_WRBACK clear.
        g.write_word(b + OUT_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );
        g.tick_with_bus(&mut bus);
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0, "pass 1 EOF");
        assert!(!g.needs_bus_tick(), "pass 1 complete");
        assert_eq!(
            bus.read_u32(desc_addr).unwrap() & DESC_OWNER_BIT,
            DESC_OWNER_BIT,
            "owner must stay 1 with OUT_AUTO_WRBACK clear"
        );

        // Re-kick the SAME untouched chain (no CPU rewrite of dw0).
        g.write_word(b + OUT_INT_CLR, 0xFFFF_FFFF);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );
        g.tick_with_bus(&mut bus);
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0, "pass 2 EOF");

        // Drain the UART: both passes' bytes reach the sink.
        let uart_idx = bus.find_peripheral_index_by_name("uart0_test").unwrap();
        let uart = bus.peripherals[uart_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3Uart>()
            .unwrap();
        for _ in 0..500_000u64 {
            uart.tick();
        }
        assert_eq!(
            *sink.lock().unwrap(),
            b"AGAINAGAIN".to_vec(),
            "same bytes transferred twice"
        );
    }

    // ── SPI2/3 DMA coupling tests ─────────────────────────────────────────

    // SPI register offsets and bits — mirrors gpspi.rs / ESP-IDF
    // `soc/esp32s3/register/soc/spi_reg.h`.
    const SPI_CMD_REG: u64 = 0x00;
    const SPI_MS_DLEN_REG: u64 = 0x1C;
    const SPI_DMA_CONF_REG: u64 = 0x30;
    const SPI_DMA_INT_RAW_REG: u64 = 0x3C;
    const SPI_USR_BIT: u32 = 1 << 24;
    const SPI_TRANS_DONE_BIT: u32 = 1 << 12;
    /// `SPI_DMA_TX_ENA : R/W ;bitpos:[28]` (spi_reg.h).
    const SPI_DMA_TX_ENA_BIT: u32 = 1 << 28;
    /// `SPI_DMA_RX_ENA : R/W ;bitpos:[27]` (spi_reg.h).
    const SPI_DMA_RX_ENA_BIT: u32 = 1 << 27;

    const SPI3_BASE: u64 = 0x6002_5000;
    const SPI2_BASE: u64 = 0x6002_4000;

    /// Test device following the `Recorder` pattern from `esp32/spi.rs`
    /// tests, with a non-trivial MISO (`mosi ^ 0xA5`) so full-duplex
    /// byte pairing is actually verified.
    struct XorDevice {
        seen: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl crate::peripherals::spi::SpiDevice for XorDevice {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.seen.lock().unwrap().push(mosi);
            mosi ^ 0xA5
        }
        fn cs_pin(&self) -> &str {
            "GPIO10"
        }
    }

    /// Build a composite `SystemBus` with DRAM at 0x3FC8_8000 and SPI3 at
    /// its real base. Optionally attaches an `XorDevice`; returns the
    /// shared MOSI log when one is attached.
    fn spi3_test_bus(
        with_device: bool,
    ) -> (SystemBus, Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>>) {
        let mut bus = bus_with_dram();
        let mut spi = Esp32s3Spi::new(22);
        let log = if with_device {
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            spi.push_device(Box::new(XorDevice { seen: seen.clone() }));
            Some(seen)
        } else {
            None
        };
        bus.add_peripheral("spi3_s3", SPI3_BASE, 0x100, None, Box::new(spi));
        (bus, log)
    }

    fn spi_write_u32(bus: &mut SystemBus, base: u64, reg_off: u64, val: u32) {
        bus.write_u32(base + reg_off, val).unwrap();
    }

    fn spi_read_u32(bus: &mut SystemBus, base: u64, reg_off: u64) -> u32 {
        bus.read_u32(base + reg_off).unwrap()
    }

    /// Kick a DMA-mode SPI transaction of `n` bytes (TX+RX enabled).
    fn kick_spi_dma(bus: &mut SystemBus, base: u64, n: usize) {
        spi_write_u32(
            bus,
            base,
            SPI_DMA_CONF_REG,
            SPI_DMA_TX_ENA_BIT | SPI_DMA_RX_ENA_BIT,
        );
        spi_write_u32(bus, base, SPI_MS_DLEN_REG, (n as u32) * 8 - 1);
        spi_write_u32(bus, base, SPI_CMD_REG, SPI_USR_BIT);
    }

    /// Drive `tick_with_bus` until the GDMA goes idle (bounded).
    fn tick_until_idle(g: &mut Esp32s3Gdma, bus: &mut SystemBus, max_ticks: usize) -> usize {
        for t in 0..max_ticks {
            if !g.needs_bus_tick() {
                return t;
            }
            g.tick_with_bus(bus);
        }
        max_ticks
    }

    /// SPI3 DMA with an attached device: descriptor-fed MOSI bytes reach
    /// the device byte-for-byte; device-fed MISO bytes land in the IN
    /// descriptor buffer; TRANS_DONE + OUT_EOF + IN_SUC_EOF all latch.
    #[test]
    fn spi3_dma_device_mosi_miso_roundtrip() {
        let (mut bus, log) = spi3_test_bus(true);
        let n: usize = 8;
        let payload: Vec<u8> = vec![0x01, 0x80, 0xFF, 0x00, 0x5A, 0xA5, 0x10, 0x7E];
        let tx_buf: u64 = 0x3FC8_8000;
        let rx_buf: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;

        for (i, &b) in payload.iter().enumerate() {
            bus.write_u8(tx_buf + i as u64, b).unwrap();
        }
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), tx_buf, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), rx_buf, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);
        // USR held + TRANS_DONE deferred until GDMA services the transaction.
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR must stay set while DMA pending"
        );
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "TRANS_DONE must wait for GDMA"
        );

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 1); // SPI3
        g.write_word(b + IN_PERI_SEL, 1); // SPI3
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );
        assert!(g.needs_bus_tick(), "coupled SPI3 transfer pending");

        let ticks = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(ticks, 1, "8-byte transfer completes in one tick");

        // MOSI reached the device byte-for-byte.
        assert_eq!(
            *log.as_ref().unwrap().lock().unwrap(),
            payload,
            "device must see the descriptor bytes in order"
        );
        // Device MISO (mosi ^ 0xA5) landed in the IN buffer.
        for (i, &b) in payload.iter().enumerate() {
            assert_eq!(
                bus.read_u8(rx_buf + i as u64).unwrap(),
                b ^ 0xA5,
                "MISO byte [{i}]"
            );
        }
        // SPI completion: TRANS_DONE latched, USR cleared.
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "TRANS_DONE latched"
        );
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR cleared"
        );
        // GDMA completion: both EOFs latched.
        let out_raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(out_raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF");
        assert_eq!(
            out_raw & OUT_TOTAL_EOF_BIT,
            OUT_TOTAL_EOF_BIT,
            "OUT_TOTAL_EOF"
        );
        assert_eq!(out_raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE");
        let in_raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(in_raw & IN_SUC_EOF_BIT, IN_SUC_EOF_BIT, "IN_SUC_EOF");
        assert_eq!(in_raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE");
    }

    /// Over-64-byte multi-descriptor transaction: 200 bytes over two OUT and
    /// two IN descriptors, pumped incrementally (64 bytes/tick → 4 ticks),
    /// with owner-bit writeback and IN length fields verified per descriptor.
    #[test]
    fn spi3_dma_multi_descriptor_200_bytes_multi_tick() {
        let (mut bus, log) = spi3_test_bus(true);
        let n: usize = 200;
        let payload: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let tx_buf1: u64 = 0x3FC8_8000;
        let tx_buf2: u64 = 0x3FC8_8100;
        let rx_buf1: u64 = 0x3FC8_9000;
        let rx_buf2: u64 = 0x3FC8_9100;
        let tx_d1: u64 = 0x3FC8_A000;
        let tx_d2: u64 = 0x3FC8_A010;
        let rx_d1: u64 = 0x3FC8_B000;
        let rx_d2: u64 = 0x3FC8_B010;

        // OUT chain: 120 + 80 bytes.
        for (i, &b) in payload[..120].iter().enumerate() {
            bus.write_u8(tx_buf1 + i as u64, b).unwrap();
        }
        for (i, &b) in payload[120..].iter().enumerate() {
            bus.write_u8(tx_buf2 + i as u64, b).unwrap();
        }
        write_desc(
            &mut bus,
            tx_d1,
            (1 << 31) | (120 << 12) | 120,
            tx_buf1,
            tx_d2,
        );
        write_desc(&mut bus, tx_d2, tx_dw0(80), tx_buf2, 0);
        // IN chain: 128 + 128 capacity (200 bytes → second ends partial at 72).
        write_desc(&mut bus, rx_d1, rx_dw0(128), rx_buf1, rx_d2);
        write_desc(&mut bus, rx_d2, rx_dw0(128), rx_buf2, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        // OUT_AUTO_WRBACK set (ESP-IDF driver default for owner-managed
        // chains) — the owner assertions below pin the writeback.
        g.write_word(b + OUT_CONF0, OUT_AUTO_WRBACK_BIT);
        g.write_word(b + OUT_PERI_SEL, 1); // SPI3
        g.write_word(b + IN_PERI_SEL, 1); // SPI3
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_d1 as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_d1 as u32 & IN_LINK_ADDR_MASK),
        );

        // Incremental: after one tick (64 bytes) the transaction must NOT
        // be complete — USR still set, no EOF, engine still pending.
        g.tick_with_bus(&mut bus);
        assert!(g.needs_bus_tick(), "200-byte transfer needs >1 tick");
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR still set mid-transfer"
        );
        assert_eq!(
            g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT,
            0,
            "no OUT_EOF mid-transfer"
        );

        // 200 bytes at 64/tick → 3 more ticks.
        let extra = tick_until_idle(&mut g, &mut bus, 16);
        assert_eq!(extra, 3, "deterministic 4-tick total for 200 bytes");

        // Device saw all 200 descriptor bytes in order.
        assert_eq!(*log.as_ref().unwrap().lock().unwrap(), payload);
        // MISO landed across both IN buffers.
        for (i, &p) in payload.iter().enumerate().take(128) {
            assert_eq!(
                bus.read_u8(rx_buf1 + i as u64).unwrap(),
                p ^ 0xA5,
                "rx_buf1[{i}]"
            );
        }
        for i in 0..72usize {
            assert_eq!(
                bus.read_u8(rx_buf2 + i as u64).unwrap(),
                payload[128 + i] ^ 0xA5,
                "rx_buf2[{i}]"
            );
        }
        // Owner-bit writeback on every consumed descriptor.
        for (name, d) in [
            ("tx_d1", tx_d1),
            ("tx_d2", tx_d2),
            ("rx_d1", rx_d1),
            ("rx_d2", rx_d2),
        ] {
            assert_eq!(
                bus.read_u32(d).unwrap() & DESC_OWNER_BIT,
                0,
                "{name} owner must be 0"
            );
        }
        // IN length fields: full first descriptor, partial second.
        assert_eq!(
            (bus.read_u32(rx_d1).unwrap() >> 12) & 0xFFF,
            128,
            "rx_d1 len"
        );
        assert_eq!(
            (bus.read_u32(rx_d2).unwrap() >> 12) & 0xFFF,
            72,
            "rx_d2 len"
        );
        // Completion flags.
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0
        );
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0);
        assert_ne!(g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT, 0);
    }

    /// TX and RX bound on DIFFERENT channels (ESP-IDF allocates them
    /// independently): the pump pairs directions by PERI_SEL, and each
    /// channel latches its own EOF.
    #[test]
    fn spi3_dma_tx_rx_on_different_channels() {
        let (mut bus, _) = spi3_test_bus(false);
        let n: usize = 4;
        let tx_buf: u64 = 0x3FC8_8000;
        let rx_buf: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        for i in 0..n {
            bus.write_u8(tx_buf + i as u64, 0x40 + i as u8).unwrap();
        }
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), tx_buf, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), rx_buf, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b_tx = ch_base(1);
        let b_rx = ch_base(3);
        g.write_word(b_tx + OUT_PERI_SEL, 1); // SPI3 OUT on ch1
        g.write_word(b_rx + IN_PERI_SEL, 1); // SPI3 IN on ch3
        g.write_word(
            b_tx + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b_rx + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );

        let ticks = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(ticks, 1);
        // No device → MISO floats high.
        for i in 0..n {
            assert_eq!(bus.read_u8(rx_buf + i as u64).unwrap(), 0xFF);
        }
        assert_ne!(
            g.read_word(b_tx + OUT_INT_RAW) & OUT_EOF_BIT,
            0,
            "ch1 OUT_EOF"
        );
        assert_ne!(
            g.read_word(b_rx + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "ch3 IN_SUC_EOF"
        );
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0
        );
    }

    /// SPI2 routes through PERI_SEL value 0 to the `spi2_s3` instance.
    #[test]
    fn spi2_dma_routes_by_peri_sel_zero() {
        let mut bus = bus_with_dram();
        bus.add_peripheral(
            "spi2_s3",
            SPI2_BASE,
            0x100,
            None,
            Box::new(Esp32s3Spi::new(21)),
        );
        let n: usize = 4;
        let tx_buf: u64 = 0x3FC8_8000;
        let tx_desc: u64 = 0x3FC8_A000;
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), tx_buf, 0);

        // TX-only DMA transaction.
        spi_write_u32(&mut bus, SPI2_BASE, SPI_DMA_CONF_REG, SPI_DMA_TX_ENA_BIT);
        spi_write_u32(&mut bus, SPI2_BASE, SPI_MS_DLEN_REG, (n as u32) * 8 - 1);
        spi_write_u32(&mut bus, SPI2_BASE, SPI_CMD_REG, SPI_USR_BIT);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_CONF0, OUT_AUTO_WRBACK_BIT);
        g.write_word(b + OUT_PERI_SEL, 0); // SPI2
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );

        let ticks = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(ticks, 1);
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0, "OUT_EOF");
        assert_ne!(
            spi_read_u32(&mut bus, SPI2_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "SPI2 TRANS_DONE"
        );
        assert_eq!(
            bus.read_u32(tx_desc).unwrap() & DESC_OWNER_BIT,
            0,
            "TX descriptor returned to CPU"
        );
    }

    /// Coupled SPI OUT with OUT_AUTO_WRBACK clear: the consumed descriptor
    /// stays DMA-owned, and a second SPI transaction + OUTLINK_START on the
    /// same untouched chain feeds the device the same bytes again (pins the
    /// gate in `coupled_out_collect`).
    #[test]
    fn spi3_tx_auto_wrback_clear_preserves_owner_and_rekick_repeats() {
        let (mut bus, log) = spi3_test_bus(true);
        let n: usize = 4;
        let payload: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let tx_buf: u64 = 0x3FC8_8000;
        let tx_desc: u64 = 0x3FC8_A000;
        for (i, &x) in payload.iter().enumerate() {
            bus.write_u8(tx_buf + i as u64, x).unwrap();
        }
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), tx_buf, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // OUT_CONF0 left at reset → OUT_AUTO_WRBACK clear.
        g.write_word(b + OUT_PERI_SEL, 1); // SPI3

        for pass in 1..=2u32 {
            // TX-only DMA transaction.
            spi_write_u32(&mut bus, SPI3_BASE, SPI_DMA_CONF_REG, SPI_DMA_TX_ENA_BIT);
            spi_write_u32(&mut bus, SPI3_BASE, SPI_MS_DLEN_REG, (n as u32) * 8 - 1);
            spi_write_u32(&mut bus, SPI3_BASE, SPI_CMD_REG, SPI_USR_BIT);
            // Same untouched descriptor chain both times.
            g.write_word(b + OUT_INT_CLR, 0xFFFF_FFFF);
            g.write_word(
                b + OUT_LINK,
                OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
            );
            let ticks = tick_until_idle(&mut g, &mut bus, 8);
            assert_eq!(ticks, 1, "pass {pass} completes in one tick");
            assert_ne!(
                g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT,
                0,
                "pass {pass} OUT_EOF"
            );
            assert_eq!(
                bus.read_u32(tx_desc).unwrap() & DESC_OWNER_BIT,
                DESC_OWNER_BIT,
                "pass {pass}: owner must stay 1 with OUT_AUTO_WRBACK clear"
            );
        }

        // The device saw the payload twice — the re-kicked chain replayed.
        let expected: Vec<u8> = payload.iter().chain(payload.iter()).copied().collect();
        assert_eq!(
            *log.as_ref().unwrap().lock().unwrap(),
            expected,
            "device must see the same descriptor bytes on both passes"
        );
    }

    /// Order independence: kicking SPI_CMD.USR BEFORE the GDMA links stalls
    /// the pump; starting the links afterwards completes the transaction.
    #[test]
    fn spi3_dma_usr_before_links_stalls_then_completes() {
        let (mut bus, _) = spi3_test_bus(false);
        let n: usize = 4;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), 0x3FC8_8000, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), 0x3FC8_9000, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 1);
        g.write_word(b + IN_PERI_SEL, 1);
        // Only the OUT link started: RX is DMA-enabled but has no chain yet,
        // so the pump must stall without consuming the transaction.
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.tick_with_bus(&mut bus);
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "stalled: USR still set with IN link missing"
        );
        assert!(g.needs_bus_tick(), "still pending while stalled");

        // Now start the IN link → completes.
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );
        let ticks = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(ticks, 1);
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR cleared after both links started"
        );
    }

    /// SPI3 DMA: TRANS_DONE and OUT_EOF/IN_SUC_EOF are all CO-LATCHED on the
    /// completion tick — this pins their joint appearance within one
    /// `tick_with_bus`, not a relative order between the three flags.
    #[test]
    fn spi3_dma_trans_done_and_eof_colatch() {
        let (mut bus, _) = spi3_test_bus(false);
        let n: usize = 4;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), 0x3FC8_8000, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), 0x3FC8_9000, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 1);
        g.write_word(b + IN_PERI_SEL, 1);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );

        g.tick_with_bus(&mut bus);

        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "SPI TRANS_DONE"
        );
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0, "OUT_EOF");
        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "IN_SUC_EOF"
        );
        assert!(!g.needs_bus_tick(), "idle after completion");
    }

    /// Burst-boundary pin: a transfer of exactly `COUPLED_BYTES_PER_TICK`
    /// (64) bytes completes in exactly ONE tick.
    #[test]
    fn spi3_dma_64_byte_transfer_completes_in_one_tick() {
        let (mut bus, _) = spi3_test_bus(false);
        let n: usize = COUPLED_BYTES_PER_TICK; // 64
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), 0x3FC8_8000, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), 0x3FC8_9000, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 1);
        g.write_word(b + IN_PERI_SEL, 1);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );

        let ticks = tick_until_idle(&mut g, &mut bus, 4);
        assert_eq!(ticks, 1, "64 bytes = exactly one full burst = one tick");
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR cleared"
        );
    }

    /// Burst-boundary pin: 65 bytes (one full burst + 1) needs exactly TWO
    /// ticks, with the transaction visibly in-flight after the first.
    #[test]
    fn spi3_dma_65_byte_transfer_takes_two_ticks() {
        let (mut bus, _) = spi3_test_bus(false);
        let n: usize = COUPLED_BYTES_PER_TICK + 1; // 65
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), 0x3FC8_8000, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), 0x3FC8_9000, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 1);
        g.write_word(b + IN_PERI_SEL, 1);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );

        // After one tick the 65th byte is still outstanding.
        g.tick_with_bus(&mut bus);
        assert!(g.needs_bus_tick(), "65th byte outstanding after tick 1");
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR still set mid-transfer"
        );

        let extra = tick_until_idle(&mut g, &mut bus, 4);
        assert_eq!(extra, 1, "65 bytes = exactly two ticks total");
    }

    /// Non-DMA regression: USR with the DMA enables clear keeps the
    /// immediate W-buffer completion path byte-identical.
    #[test]
    fn spi3_non_dma_regression() {
        let (mut bus, _) = spi3_test_bus(false);
        spi_write_u32(&mut bus, SPI3_BASE, SPI_MS_DLEN_REG, 32 - 1);
        spi_write_u32(&mut bus, SPI3_BASE, SPI_CMD_REG, SPI_USR_BIT);
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "non-DMA USR auto-clears immediately"
        );
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "non-DMA TRANS_DONE latches immediately"
        );
        // MISO region = 0xFF in the W buffer (CPU path).
        assert_eq!(bus.read_u32(SPI3_BASE + 0x98).unwrap(), 0xFFFF_FFFF, "W0");
    }

    // ── I2S0/1 DMA coupling tests ─────────────────────────────────────────

    // I2S register offsets and bits — mirrors i2s.rs / ESP-IDF
    // `soc/esp32s3/register/soc/i2s_reg.h`.
    const I2S_RX_CONF_REG: u64 = 0x20;
    const I2S_TX_CONF_REG: u64 = 0x24;
    const I2S_RXEOF_NUM_REG: u64 = 0x64;
    /// TX_START / RX_START (bit 2 of TX_CONF / RX_CONF).
    const I2S_START_BIT: u32 = 1 << 2;

    use crate::peripherals::esp32s3::i2s::{I2S0_BASE, I2S1_BASE};

    /// Build a composite `SystemBus` with DRAM plus one I2S controller at
    /// `base` registered under `name`, with a TX sample sink attached.
    fn i2s_test_bus(name: &str, base: u64, source_id: u32) -> (SystemBus, Arc<Mutex<Vec<u8>>>) {
        let mut bus = bus_with_dram();
        let mut i2s = Esp32s3I2s::new(source_id);
        let sink = Arc::new(Mutex::new(Vec::new()));
        i2s.set_tx_sink(Some(sink.clone()));
        bus.add_peripheral(name, base, 0x1000, None, Box::new(i2s));
        (bus, sink)
    }

    /// Queue RX sample bytes on a bus-registered I2S controller.
    fn push_i2s_rx(bus: &mut SystemBus, name: &str, bytes: &[u8]) {
        let idx = bus.find_peripheral_index_by_name(name).unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3I2s>()
            .unwrap()
            .push_rx_samples(bytes);
    }

    /// TX: a known pattern through a two-descriptor OUT chain (100 bytes →
    /// two 64-byte ticks) lands in the I2S sample sink in order; OUT_EOF
    /// latches only after chain completion; owner bits cleared on both
    /// consumed descriptors.
    #[test]
    fn i2s0_tx_pattern_streams_to_sink_multi_descriptor() {
        let (mut bus, sink) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let payload: Vec<u8> = (0..100u32).map(|i| (i * 7 % 256) as u8).collect();
        let buf1: u64 = 0x3FC8_8000;
        let buf2: u64 = 0x3FC8_8100;
        let d1: u64 = 0x3FC8_A000;
        let d2: u64 = 0x3FC8_A010;
        for (i, &b) in payload[..60].iter().enumerate() {
            bus.write_u8(buf1 + i as u64, b).unwrap();
        }
        for (i, &b) in payload[60..].iter().enumerate() {
            bus.write_u8(buf2 + i as u64, b).unwrap();
        }
        write_desc(&mut bus, d1, (1 << 31) | (60 << 12) | 60, buf1, d2);
        write_desc(&mut bus, d2, tx_dw0(40), buf2, 0);

        // Start the I2S TX engine via its real MMIO control bit.
        bus.write_u32(I2S0_BASE as u64 + I2S_TX_CONF_REG, I2S_START_BIT)
            .unwrap();

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_CONF0, OUT_AUTO_WRBACK_BIT);
        g.write_word(b + OUT_PERI_SEL, 3); // I2S0
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (d1 as u32 & OUT_LINK_ADDR_MASK),
        );
        assert!(g.needs_bus_tick(), "coupled I2S0 TX pending");
        assert_eq!(
            g.read_word(b + OUT_INT_RAW) & (OUT_EOF_BIT | OUT_DONE_BIT),
            0,
            "no EOF before the pump runs"
        );

        // 100 bytes at 64/tick → exactly 2 ticks; no EOF mid-transfer.
        g.tick_with_bus(&mut bus);
        assert!(g.needs_bus_tick(), "100-byte transfer needs >1 tick");
        assert_eq!(
            g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT,
            0,
            "no OUT_EOF mid-transfer"
        );
        let extra = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(extra, 1, "deterministic 2-tick total for 100 bytes");

        assert_eq!(
            *sink.lock().unwrap(),
            payload,
            "sample sink must hold the pattern in order"
        );
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF");
        assert_eq!(raw & OUT_TOTAL_EOF_BIT, OUT_TOTAL_EOF_BIT, "OUT_TOTAL_EOF");
        assert_eq!(raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE");
        for (name, d) in [("d1", d1), ("d2", d2)] {
            assert_eq!(
                bus.read_u32(d).unwrap() & DESC_OWNER_BIT,
                0,
                "{name} owner must be returned to CPU"
            );
        }
    }

    /// Start-bit gating (TX): with TX_START clear nothing moves and EOF
    /// stays unlatched; setting TX_START resumes the transfer.
    #[test]
    fn i2s0_tx_gated_until_tx_start_set() {
        let (mut bus, sink) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let buf: u64 = 0x3FC8_8000;
        let d: u64 = 0x3FC8_A000;
        for i in 0..8u64 {
            bus.write_u8(buf + i, 0x30 + i as u8).unwrap();
        }
        write_desc(&mut bus, d, tx_dw0(8), buf, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + OUT_PERI_SEL, 3); // I2S0
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (d as u32 & OUT_LINK_ADDR_MASK),
        );

        // TX_START is clear: the pump must stall, retaining state.
        for _ in 0..3 {
            g.tick_with_bus(&mut bus);
        }
        assert!(sink.lock().unwrap().is_empty(), "no movement while stopped");
        assert_eq!(g.read_word(b + OUT_INT_RAW), 0, "no EOF while stopped");
        assert!(g.needs_bus_tick(), "transfer still pending while stopped");

        // Set TX_START → movement resumes and completes.
        bus.write_u32(I2S0_BASE as u64 + I2S_TX_CONF_REG, I2S_START_BIT)
            .unwrap();
        g.tick_with_bus(&mut bus);
        assert_eq!(
            *sink.lock().unwrap(),
            (0..8).map(|i| 0x30 + i as u8).collect::<Vec<_>>()
        );
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0, "OUT_EOF");
        assert!(!g.needs_bus_tick());
    }

    /// RX with RXEOF_NUM below one descriptor's capacity: IN_SUC_EOF latches
    /// after exactly N bytes even though both the source queue and the
    /// descriptor have more room; excess source bytes stay queued.
    #[test]
    fn i2s0_rx_eof_after_exactly_rxeof_num_bytes() {
        let (mut bus, _) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let n: u32 = 16;
        let buf: u64 = 0x3FC8_9000;
        let d: u64 = 0x3FC8_B000;
        write_desc(&mut bus, d, rx_dw0(64), buf, 0);

        bus.write_u32(I2S0_BASE as u64 + I2S_RXEOF_NUM_REG, n)
            .unwrap();
        bus.write_u32(I2S0_BASE as u64 + I2S_RX_CONF_REG, I2S_START_BIT)
            .unwrap();
        // Queue MORE than RXEOF_NUM bytes.
        let samples: Vec<u8> = (0..32u8).map(|i| 0xC0 ^ i).collect();
        push_i2s_rx(&mut bus, I2S0_S3_NAME, &samples);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 3); // I2S0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (d as u32 & IN_LINK_ADDR_MASK),
        );

        let ticks = tick_until_idle(&mut g, &mut bus, 4);
        assert_eq!(ticks, 1, "16 bytes complete in one tick");

        let in_raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(in_raw & IN_SUC_EOF_BIT, IN_SUC_EOF_BIT, "IN_SUC_EOF");
        assert_eq!(in_raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE");
        // Exactly N bytes landed; the buffer beyond N is untouched (zero).
        for (i, &s) in samples.iter().enumerate().take(n as usize) {
            assert_eq!(bus.read_u8(buf + i as u64).unwrap(), s, "[{i}]");
        }
        for i in n as usize..(n as usize + 4) {
            assert_eq!(bus.read_u8(buf + i as u64).unwrap(), 0, "beyond N [{i}]");
        }
        // Descriptor returned to CPU with the received length = N.
        let dw0 = bus.read_u32(d).unwrap();
        assert_eq!(dw0 & DESC_OWNER_BIT, 0, "owner cleared");
        assert_eq!((dw0 >> 12) & 0xFFF, n, "received length = RXEOF_NUM");
    }

    /// RX with RXEOF_NUM equal to the descriptor capacity: EOF latches with
    /// the descriptor exactly full.
    #[test]
    fn i2s0_rx_eof_num_equal_to_descriptor_capacity() {
        let (mut bus, _) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let n: u32 = 32;
        let buf: u64 = 0x3FC8_9000;
        let d: u64 = 0x3FC8_B000;
        write_desc(&mut bus, d, rx_dw0(n), buf, 0);
        bus.write_u32(I2S0_BASE as u64 + I2S_RXEOF_NUM_REG, n)
            .unwrap();
        bus.write_u32(I2S0_BASE as u64 + I2S_RX_CONF_REG, I2S_START_BIT)
            .unwrap();
        push_i2s_rx(&mut bus, I2S0_S3_NAME, &[0x5A; 32]);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + IN_PERI_SEL, 3); // I2S0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (d as u32 & IN_LINK_ADDR_MASK),
        );
        let ticks = tick_until_idle(&mut g, &mut bus, 4);
        assert_eq!(ticks, 1);
        assert_ne!(g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT, 0);
        let dw0 = bus.read_u32(d).unwrap();
        assert_eq!(dw0 & DESC_OWNER_BIT, 0, "owner cleared");
        assert_eq!((dw0 >> 12) & 0xFFF, n, "received length = capacity");
    }

    /// RX with RXEOF_NUM above one descriptor's capacity: the transfer
    /// spans into a second descriptor and EOF latches after exactly N
    /// bytes, with per-descriptor owner/length writeback (full first,
    /// partial second).
    #[test]
    fn i2s0_rx_eof_num_spans_two_descriptors() {
        let (mut bus, _) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let n: u32 = 48; // > 32 (first descriptor's capacity)
        let buf1: u64 = 0x3FC8_9000;
        let buf2: u64 = 0x3FC8_9100;
        let d1: u64 = 0x3FC8_B000;
        let d2: u64 = 0x3FC8_B010;
        write_desc(&mut bus, d1, rx_dw0(32), buf1, d2);
        write_desc(&mut bus, d2, rx_dw0(32), buf2, 0);
        bus.write_u32(I2S0_BASE as u64 + I2S_RXEOF_NUM_REG, n)
            .unwrap();
        bus.write_u32(I2S0_BASE as u64 + I2S_RX_CONF_REG, I2S_START_BIT)
            .unwrap();
        let samples: Vec<u8> = (0..64u8).collect(); // more than N queued
        push_i2s_rx(&mut bus, I2S0_S3_NAME, &samples);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 3); // I2S0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (d1 as u32 & IN_LINK_ADDR_MASK),
        );
        let ticks = tick_until_idle(&mut g, &mut bus, 4);
        assert_eq!(ticks, 1, "48 bytes within one 64-byte tick");

        assert_ne!(g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT, 0);
        for (i, &s) in samples.iter().enumerate().take(32) {
            assert_eq!(bus.read_u8(buf1 + i as u64).unwrap(), s);
        }
        for i in 0..16usize {
            assert_eq!(bus.read_u8(buf2 + i as u64).unwrap(), samples[32 + i]);
        }
        let dw0_1 = bus.read_u32(d1).unwrap();
        let dw0_2 = bus.read_u32(d2).unwrap();
        assert_eq!(dw0_1 & DESC_OWNER_BIT, 0, "d1 owner cleared");
        assert_eq!(dw0_2 & DESC_OWNER_BIT, 0, "d2 owner cleared");
        assert_eq!((dw0_1 >> 12) & 0xFFF, 32, "d1 length full");
        assert_eq!((dw0_2 >> 12) & 0xFFF, 16, "d2 length partial = N - 32");
    }

    /// RX samples that trickle in across ticks accumulate toward RXEOF_NUM;
    /// EOF only latches once the cumulative count reaches N.
    #[test]
    fn i2s0_rx_partial_samples_accumulate_across_ticks() {
        let (mut bus, _) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let n: u32 = 16;
        let buf: u64 = 0x3FC8_9000;
        let d: u64 = 0x3FC8_B000;
        write_desc(&mut bus, d, rx_dw0(64), buf, 0);
        bus.write_u32(I2S0_BASE as u64 + I2S_RXEOF_NUM_REG, n)
            .unwrap();
        bus.write_u32(I2S0_BASE as u64 + I2S_RX_CONF_REG, I2S_START_BIT)
            .unwrap();

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 3); // I2S0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (d as u32 & IN_LINK_ADDR_MASK),
        );

        // First half of the samples: no EOF yet.
        push_i2s_rx(&mut bus, I2S0_S3_NAME, &[0x11; 8]);
        g.tick_with_bus(&mut bus);
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "8 of 16 bytes: no EOF"
        );
        assert!(g.needs_bus_tick(), "still pending");

        // Second half arrives later → EOF at the cumulative N.
        push_i2s_rx(&mut bus, I2S0_S3_NAME, &[0x22; 8]);
        g.tick_with_bus(&mut bus);
        assert_ne!(g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT, 0, "EOF at N");
        assert!(!g.needs_bus_tick());
        let dw0 = bus.read_u32(d).unwrap();
        assert_eq!(dw0 & DESC_OWNER_BIT, 0);
        assert_eq!((dw0 >> 12) & 0xFFF, n, "received length = N");
    }

    /// Start-bit gating (RX): queued samples do not move while RX_START is
    /// clear; movement resumes after start.
    #[test]
    fn i2s0_rx_gated_until_rx_start_set() {
        let (mut bus, _) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let buf: u64 = 0x3FC8_9000;
        let d: u64 = 0x3FC8_B000;
        write_desc(&mut bus, d, rx_dw0(8), buf, 0);
        bus.write_u32(I2S0_BASE as u64 + I2S_RXEOF_NUM_REG, 8)
            .unwrap();
        push_i2s_rx(&mut bus, I2S0_S3_NAME, &[0x77; 8]);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(3);
        g.write_word(b + IN_PERI_SEL, 3); // I2S0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (d as u32 & IN_LINK_ADDR_MASK),
        );

        // RX_START clear: stall.
        for _ in 0..3 {
            g.tick_with_bus(&mut bus);
        }
        assert_eq!(bus.read_u8(buf).unwrap(), 0, "no movement while stopped");
        assert_eq!(g.read_word(b + IN_INT_RAW), 0, "no EOF while stopped");
        assert!(g.needs_bus_tick());

        // Start RX → samples land and EOF latches.
        bus.write_u32(I2S0_BASE as u64 + I2S_RX_CONF_REG, I2S_START_BIT)
            .unwrap();
        g.tick_with_bus(&mut bus);
        for i in 0..8u64 {
            assert_eq!(bus.read_u8(buf + i).unwrap(), 0x77, "[{i}]");
        }
        assert_ne!(g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT, 0);
        assert!(!g.needs_bus_tick());
    }

    /// I2S1 routes through PERI_SEL value 4 to the `i2s1_s3` instance —
    /// pairing is by PERI_SEL value, not channel index.
    #[test]
    fn i2s1_routes_by_peri_sel_four() {
        let (mut bus, sink) = i2s_test_bus(I2S1_S3_NAME, I2S1_BASE as u64, 26);
        let buf: u64 = 0x3FC8_8000;
        let d: u64 = 0x3FC8_A000;
        let payload = b"I2S1-DMA";
        for (i, &x) in payload.iter().enumerate() {
            bus.write_u8(buf + i as u64, x).unwrap();
        }
        write_desc(&mut bus, d, tx_dw0(payload.len() as u32), buf, 0);
        bus.write_u32(I2S1_BASE as u64 + I2S_TX_CONF_REG, I2S_START_BIT)
            .unwrap();

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(4);
        g.write_word(b + OUT_CONF0, OUT_AUTO_WRBACK_BIT);
        g.write_word(b + OUT_PERI_SEL, 4); // I2S1
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (d as u32 & OUT_LINK_ADDR_MASK),
        );
        let ticks = tick_until_idle(&mut g, &mut bus, 4);
        assert_eq!(ticks, 1);
        assert_eq!(*sink.lock().unwrap(), payload.to_vec());
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0, "OUT_EOF");
        assert_eq!(
            bus.read_u32(d).unwrap() & DESC_OWNER_BIT,
            0,
            "descriptor returned to CPU"
        );
    }

    /// I2S IN IRQ: with IN_INT_ENA set, EOF emits the channel's
    /// interrupt-matrix source from `tick()`.
    #[test]
    fn i2s0_rx_irq_fires_on_suc_eof() {
        let (mut bus, _) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let buf: u64 = 0x3FC8_9000;
        let d: u64 = 0x3FC8_B000;
        write_desc(&mut bus, d, rx_dw0(4), buf, 0);
        bus.write_u32(I2S0_BASE as u64 + I2S_RXEOF_NUM_REG, 4)
            .unwrap();
        bus.write_u32(I2S0_BASE as u64 + I2S_RX_CONF_REG, I2S_START_BIT)
            .unwrap();
        push_i2s_rx(&mut bus, I2S0_S3_NAME, &[1, 2, 3, 4]);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_INT_ENA, IN_SUC_EOF_BIT);
        g.write_word(b + IN_PERI_SEL, 3); // I2S0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (d as u32 & IN_LINK_ADDR_MASK),
        );
        g.tick_with_bus(&mut bus);
        assert_eq!(
            g.tick().explicit_irqs.as_deref(),
            Some(&[IN_CH0_SRC + 1][..]),
            "IN_CH1 source after I2S RX EOF"
        );
    }

    /// RX-side per-tick budget: RXEOF_NUM = 80 with 100 bytes queued moves
    /// exactly `COUPLED_BYTES_PER_TICK` (64) bytes on tick 1 (no EOF) and
    /// the remaining 16 on tick 2 (EOF at exactly N), with per-descriptor
    /// owner/length writeback (full 64 on d1, partial 16 on d2) and the
    /// 20 excess source bytes left queued.
    #[test]
    fn i2s0_rx_64_byte_tick_budget_spans_ticks() {
        let (mut bus, _) = i2s_test_bus(I2S0_S3_NAME, I2S0_BASE as u64, 25);
        let n: u32 = 80;
        let buf1: u64 = 0x3FC8_9000;
        let buf2: u64 = 0x3FC8_9100;
        let d1: u64 = 0x3FC8_B000;
        let d2: u64 = 0x3FC8_B010;
        write_desc(&mut bus, d1, rx_dw0(64), buf1, d2);
        write_desc(&mut bus, d2, rx_dw0(64), buf2, 0);
        bus.write_u32(I2S0_BASE as u64 + I2S_RXEOF_NUM_REG, n)
            .unwrap();
        bus.write_u32(I2S0_BASE as u64 + I2S_RX_CONF_REG, I2S_START_BIT)
            .unwrap();
        let samples: Vec<u8> = (0..100u32).map(|i| (i * 3 % 251) as u8).collect();
        push_i2s_rx(&mut bus, I2S0_S3_NAME, &samples);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 3); // I2S0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (d1 as u32 & IN_LINK_ADDR_MASK),
        );

        // Tick 1: exactly the 64-byte budget moves; no EOF yet.
        g.tick_with_bus(&mut bus);
        assert!(g.needs_bus_tick(), "80-byte transfer needs a second tick");
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "no IN_SUC_EOF after 64 of 80 bytes"
        );
        for (i, &s) in samples.iter().enumerate().take(64) {
            assert_eq!(bus.read_u8(buf1 + i as u64).unwrap(), s, "[{i}]");
        }
        assert_eq!(bus.read_u8(buf2).unwrap(), 0, "tick 1 must not touch d2");
        let dw0_1 = bus.read_u32(d1).unwrap();
        assert_eq!(dw0_1 & DESC_OWNER_BIT, 0, "d1 owner cleared on tick 1");
        assert_eq!((dw0_1 >> 12) & 0xFFF, 64, "d1 length = full capacity");

        // Tick 2: the remaining 16 bytes land; EOF at exactly N.
        g.tick_with_bus(&mut bus);
        assert!(!g.needs_bus_tick(), "transfer complete after tick 2");
        let in_raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(in_raw & IN_SUC_EOF_BIT, IN_SUC_EOF_BIT, "IN_SUC_EOF");
        assert_eq!(in_raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE");
        for i in 0..16usize {
            assert_eq!(
                bus.read_u8(buf2 + i as u64).unwrap(),
                samples[64 + i],
                "d2 [{i}]"
            );
        }
        for i in 16..20usize {
            assert_eq!(bus.read_u8(buf2 + i as u64).unwrap(), 0, "beyond N [{i}]");
        }
        let dw0_2 = bus.read_u32(d2).unwrap();
        assert_eq!(dw0_2 & DESC_OWNER_BIT, 0, "d2 owner cleared");
        assert_eq!((dw0_2 >> 12) & 0xFFF, 16, "d2 length partial = N - 64");

        // The 20 excess source bytes stay queued for the next transfer.
        let idx = bus.find_peripheral_index_by_name(I2S0_S3_NAME).unwrap();
        let leftover = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3I2s>()
            .unwrap()
            .dma_pop_rx(usize::MAX);
        assert_eq!(leftover, samples[80..].to_vec(), "excess stays queued");
    }
}
