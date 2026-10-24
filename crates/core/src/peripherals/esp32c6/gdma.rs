// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GDMA (General DMA) controller for ESP32-C6.
//!
//! Base = `DR_REG_GDMA_BASE` = `0x6008_0000` (esp-idf v5.3 `soc/reg_base.h`);
//! the vendored `esp32c6.svd` declares the block as `DMA` at the same base.
//! The C6 GDMA is a newer revision of the S3 GDMA: **three channel pairs**
//! (IN/RX + OUT/TX per channel, i.e. six directions), and a register file whose
//! layout is NOT the S3's flat `0xC0`-stride block — the C6 interleaves
//! per-direction INT registers at the bottom of the window with per-channel
//! config blocks, and `OUT_CONF0_CHn` sits at `0x190 + n*0xC0`, outside the
//! `0xC0` stride (so `OUT_CONF0_CH0` is numerically *inside* channel 1's
//! block). Every offset below is taken from
//! `tests/fixtures/real_world/esp32c6.svd`:
//!
//! | Flat offset            | Register (channel stride)                          |
//! |-----------------------:|----------------------------------------------------|
//! | 0x00 + n*0x10          | `IN_INT_RAW_CHn` / `_ST` / `_ENA` / `_CLR` (+0/4/8/C) |
//! | 0x30 + n*0x10          | `OUT_INT_RAW_CHn` / `_ST` / `_ENA` / `_CLR`          |
//! | 0x60 / 0x64 / 0x68     | `AHB_TEST` / `MISC_CONF` / `DATE`                  |
//! | 0x70 + n*0xC0          | `IN_CONF0` (+0x00), `IN_CONF1` (+0x04),             |
//! |                        | `INFIFO_STATUS` (+0x08), `IN_POP` (+0x0C),          |
//! |                        | `IN_LINK` (+0x10), `IN_STATE` (+0x14),              |
//! |                        | `IN_SUC_EOF_DES_ADDR` (+0x18), `IN_ERR_EOF_DES_ADDR` (+0x1C), |
//! |                        | `IN_DSCR`..`IN_DSCR_BF1` (+0x20..0x28), `IN_PRI` (+0x2C), |
//! |                        | `IN_PERI_SEL` (+0x30)                               |
//! | 0xD4 + n*0xC0          | `OUT_CONF1` (+0x00), `OUTFIFO_STATUS` (+0x04),      |
//! |                        | `OUT_PUSH` (+0x08), `OUT_LINK` (+0x0C),             |
//! |                        | `OUT_STATE` (+0x10), `OUT_EOF_DES_ADDR` (+0x14),    |
//! |                        | `OUT_EOF_BFR_DES_ADDR` (+0x18),                     |
//! |                        | `OUT_DSCR`..`OUT_DSCR_BF1` (+0x1C..0x24), `OUT_PRI` (+0x28), |
//! |                        | `OUT_PERI_SEL` (+0x2C)                              |
//! | 0x190 + n*0xC0         | `OUT_CONF0`                                        |
//!
//! ## What is modelled — memory-to-memory transfers
//!
//! Setting `IN_CONF0.MEM_TRANS_EN` (bit 4) and then starting both `IN_LINK`
//! and `OUT_LINK` on the **same channel** arms a real descriptor walk, run in
//! `tick_with_bus` (C6 TRM §3.4.3: "a transmit channel is only connected to
//! the receive channel with the same number (n) ... PERI_IN_SEL and
//! PERI_OUT_SEL should be configured to the same value corresponding to
//! Dummy"): the OUT chain's buffers are read and their bytes written into the
//! IN chain's buffers, `IN_SUC_EOF | IN_DONE` and
//! `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE` latch, and the last-descriptor
//! addresses are recorded in `IN_SUC_EOF_DES_ADDR` / `OUT_EOF_DES_ADDR`.
//!
//! Descriptors use the Espressif 3-word linked-list format (C6 TRM §3.4):
//! dw0 owner bit 31, suc_eof bit 30, length bits [23:12], size bits [11:0];
//! dw1 full 32-bit buffer address (must be in HP SRAM `0x4080_0000..0x4087_FFFF`
//! per the TRM); dw2 full 32-bit next-descriptor address or 0 = EOL.
//! `IN_CHECK_OWNER_CHn` / `OUT_CHECK_OWNER_CHn` (CONF1 bit 12) gate the
//! owner-bit check exactly as the TRM documents; with the bit clear a
//! CPU-owned descriptor is still walked. IN writeback on completion clears
//! the owner bit and stores the received byte count in the length field;
//! OUT descriptors are returned to the CPU only when `OUT_CONF0.OUT_AUTO_WRBACK`
//! (bit 2) is set, mirroring the S3 twin's semantics.
//!
//! Descriptor addresses: `INLINK_ADDR` / `OUTLINK_ADDR` are 20-bit fields
//! ("the lower 20 bits of the first descriptor's address", TRM §3.8). The
//! upper bits are not in the register, so the model reconstructs the bus
//! address as `0x4080_0000 | addr[19:0]` — the HP-SRAM base the C6 chip
//! descriptor declares, and the only memory window the TRM lets the DMA read
//! descriptors from.
//!
//! ## What is NOT modelled (documented stubs, never silent byte movement)
//!
//! * **Peripheral-coupled transfers** other than the GP-SPI2 (UHCI0, I2S,
//!   AES, SHA, ADC, PARLIO): a START with `MEM_TRANS_EN` clear and such a
//!   peripheral bound in `PERI_SEL` (2, 3, 6, 7, 8, 9) is marked *pending* and
//!   **stalls visibly** — no EOF latches and `needs_bus_tick()` stays true.
//!   There is no peripheral-side pump for them, and auto-completing would
//!   claim a transfer that never moved a byte. The **GP-SPI2** (`PERI_SEL` =
//!   0) IS coupled: the OUT list feeds the controller's transmit FIFO and the
//!   IN list receives what a DMA transaction read, through the C3's shared
//!   coupling ([`crate::peripherals::esp_gdma`]). A Dummy `PERI_SEL` value or the unbound
//!   reset value (`0x3F`) takes the fallback auto-complete path (EOF/DONE
//!   latch, zero bytes moved), matching the S3 twin's fallback so a firmware
//!   that never binds a peripheral cannot hang.
//! * **FIFO data ports** `IN_POP_CHn` / `OUT_PUSH_CHn`: no FIFO engine; the
//!   `*FIFO_STATUS` EMPTY bits read 1 (transfers are one-shot, nothing is
//!   buffered).
//! * **`IN_STATE` / `OUT_STATE`, priority arbitration, ETM, AHBM reset and
//!   burst tuning**: storage/zero. A walk runs to completion in one
//!   `tick_with_bus`; `IN_RST` / `OUT_RST` are not acted on.
//! * **Interrupt delivery**: RAW/ST/ENA/CLR are real and the asserted matrix
//!   sources (IN_CHn = 66+n, OUT_CHn = 69+n on the C6) are exported via
//!   `tick()` / `matrix_irq_sources_into`, but no C6 fixture drives one
//!   through the interrupt fabric yet (the tier-1 `dma` check is polled).
//! * **Descriptor errors on the memory-to-memory path**: an under-provisioned
//!   IN chain drops the excess bytes and a malformed chain just ends the walk
//!   (same documented simplification as the S3 model). The GP-SPI2 path walks
//!   with the shared walkers and reports `IN_DSCR_ERR` / `OUT_DSCR_ERR` /
//!   `IN_DSCR_EMPTY` ([`crate::peripherals::esp_gdma`]).

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};

/// Interrupt-matrix source for DMA IN channel 0 (`DMA_IN_CH0` = 66 in the
/// ESP32-C6 SVD; `DMA_IN_CH1` = 67, `DMA_IN_CH2` = 68, then OUT_CH0..2 =
/// 69..71). The chip descriptor declares it in `irq:`; this is the fallback.
pub const DMA_IN_CH0_INTR_SOURCE_ID: u32 = 66;

/// Number of GDMA channel pairs on the ESP32-C6 (three TX + three RX).
const NUM_CHANNELS: usize = 3;

/// Per-channel config-block stride (`IN_CONF0_CH1 - IN_CONF0_CH0`).
const CHANNEL_STRIDE: u64 = 0xC0;

/// Base of the per-channel IN config block (`IN_CONF0_CH0`).
const IN_BLOCK_BASE: u64 = 0x70;
/// Base of `OUT_CONF1_CHn` (it sits inside the channel's IN block at `+0x64`).
const OUT_CONF1_BASE: u64 = 0xD4;
/// Base of `OUT_CONF0_CHn` — deliberately outside the `0xC0` stride.
const OUT_CONF0_BASE: u64 = 0x190;

/// Global registers.
const AHB_TEST_OFFSET: u64 = 0x60;
const MISC_CONF_OFFSET: u64 = 0x64;
const DATE_OFFSET: u64 = 0x68;

/// Flat interrupt-register bases and per-channel stride.
const IN_INT_BASE: u64 = 0x00;
const OUT_INT_BASE: u64 = 0x30;
const INT_CHANNEL_STRIDE: u64 = 0x10;

/// Offsets of the four INT registers within one direction's flat block.
const INT_RAW: u64 = 0x0;
const INT_ST: u64 = 0x4;
const INT_ENA: u64 = 0x8;
const INT_CLR: u64 = 0xC;

// ── IN_LINK (channel block +0x10) bits ────────────────────────────────────
const LINK_ADDR_MASK: u32 = 0x000F_FFFF;
const IN_LINK_AUTO_RET_BIT: u32 = 1 << 20;
const IN_LINK_STOP_BIT: u32 = 1 << 21;
const IN_LINK_START_BIT: u32 = 1 << 22;
const IN_LINK_RESTART_BIT: u32 = 1 << 23;
const IN_LINK_PARK_BIT: u32 = 1 << 24;

// ── OUT_LINK (channel block +0x0C) bits ───────────────────────────────────
const OUT_LINK_STOP_BIT: u32 = 1 << 20;
const OUT_LINK_START_BIT: u32 = 1 << 21;
const OUT_LINK_RESTART_BIT: u32 = 1 << 22;
const OUT_LINK_PARK_BIT: u32 = 1 << 23;

/// `IN_CONF0.MEM_TRANS_EN` (bit 4) — select memory-to-memory mode.
const MEM_TRANS_EN_BIT: u32 = 1 << 4;
/// `OUT_CONF0.OUT_AUTO_WRBACK` (bit 2) — clear the owner bit on consumed OUT
/// descriptors. Reset value of `OUT_CONF0` is 0x8 (OUT_EOF_MODE=1), so the
/// writeback is off until firmware opts in, exactly like the S3 twin.
const OUT_AUTO_WRBACK_BIT: u32 = 1 << 2;
/// `IN_CONF1.IN_CHECK_OWNER` / `OUT_CONF1.OUT_CHECK_OWNER` (bit 12).
const CHECK_OWNER_BIT: u32 = 1 << 12;

// ── IN_INT_* bits ─────────────────────────────────────────────────────────
const IN_DONE_BIT: u32 = 1 << 0;
const IN_SUC_EOF_BIT: u32 = 1 << 1;
#[allow(dead_code)]
const IN_ERR_EOF_BIT: u32 = 1 << 2;
const IN_DSCR_ERR_BIT: u32 = 1 << 3;
const IN_DSCR_EMPTY_BIT: u32 = 1 << 4;

// ── OUT_INT_* bits ────────────────────────────────────────────────────────
const OUT_DONE_BIT: u32 = 1 << 0;
const OUT_EOF_BIT: u32 = 1 << 1;
const OUT_DSCR_ERR_BIT: u32 = 1 << 2;
const OUT_TOTAL_EOF_BIT: u32 = 1 << 3;

/// `*FIFO_STATUS` EMPTY bit (bit 1) — no FIFO engine, so both directions
/// report empty.
const FIFO_EMPTY_BIT: u32 = 1 << 1;

/// 6-bit mask for the PERI_SEL fields.
const PERI_SEL_MASK: u32 = 0x3F;
/// Unbound PERI_SEL reset value (no peripheral selected).
const PERI_SEL_RESET: u32 = 0x3F;

/// Full bus address prefix for descriptor fetches. `INLINK_ADDR` is a 20-bit
/// field; C6 GDMA descriptors must live in HP SRAM, which the chip descriptor
/// places at `0x4080_0000`. See the module docs for the provenance.
const DESC_ADDR_PREFIX: u32 = 0x4080_0000;

/// The HP SRAM the DMA reaches (the chip yaml's `ram`, 0x4080_0000..
/// 0x4088_0000): a descriptor, buffer or next link of a GP-SPI2 list outside
/// it is a descriptor error ([`crate::peripherals::esp_gdma`]).
const DMA_RAM: std::ops::Range<u32> = 0x4080_0000..0x4088_0000;

/// Maximum descriptor hops per chain (runaway-chain guard).
const MAX_DESC_CHAIN: usize = 4096;

/// Descriptor dw0 bit positions (Espressif 3-word linked-list format).
const DESC_OWNER_BIT: u32 = 1 << 31;
const DESC_SUC_EOF_BIT: u32 = 1 << 30;
const DESC_LEN_MASK: u32 = 0xFFF << 12;

/// One direction (IN or OUT) of one channel.
#[derive(Debug, Clone, Copy)]
struct DmaDir {
    conf0: u32,
    conf1: u32,
    /// 20-bit descriptor-list address (the LINK register's ADDR field).
    link_addr: u32,
    /// `INLINK_AUTO_RET` — stored, no behavioural effect in this model.
    link_auto_ret: bool,
    /// Sticky interrupt-pending bits (cleared only via `*_INT_CLR`).
    int_raw: u32,
    /// Per-bit interrupt enable.
    int_ena: u32,
    /// 6-bit peripheral binding; reset `0x3F` = unbound.
    peri_sel: u32,
}

impl Default for DmaDir {
    fn default() -> Self {
        Self {
            conf0: 0,
            conf1: 0,
            link_addr: 0,
            link_auto_ret: false,
            int_raw: 0,
            int_ena: 0,
            peri_sel: PERI_SEL_RESET,
        }
    }
}

/// One channel pair's transfer state.
#[derive(Debug, Clone, Copy)]
struct Channel {
    rx: DmaDir,
    tx: DmaDir,
    /// `IN_LINK` received a START while `MEM_TRANS_EN` was set.
    in_started: bool,
    /// `OUT_LINK` received a START while `MEM_TRANS_EN` was set.
    out_started: bool,
    /// Both directions started; the next `tick_with_bus` runs the walk.
    pending_m2m: bool,
    /// START with a real peripheral bound and no pump: visible stall.
    in_coupled_pending: bool,
    out_coupled_pending: bool,
    /// Last descriptor addresses recorded into `*_EOF_DES_ADDR`.
    in_suc_eof_des_addr: u32,
    out_eof_des_addr: u32,
}

impl Default for Channel {
    /// Seed the SVD reset values that live inside a channel: `IN_LINK` resets
    /// with `INLINK_AUTO_RET` set (`0x0110_0000`), `OUT_CONF0` resets with
    /// `OUT_EOF_MODE` set (`0x8`).
    fn default() -> Self {
        let rx = DmaDir {
            link_auto_ret: true,
            ..DmaDir::default()
        };
        let tx = DmaDir {
            conf0: 0x8,
            ..DmaDir::default()
        };
        Self {
            rx,
            tx,
            in_started: false,
            out_started: false,
            pending_m2m: false,
            in_coupled_pending: false,
            out_coupled_pending: false,
            in_suc_eof_des_addr: 0,
            out_eof_des_addr: 0,
        }
    }
}

/// One decoded GDMA linked-list descriptor.
#[derive(Debug, Clone, Copy)]
struct Desc {
    dw0: u32,
    len: u32,
    size: u32,
    buf: u64,
    next: u64,
}

impl Desc {
    fn read(bus: &mut dyn Bus, addr: u64) -> Self {
        let (dw0, buf, next) = crate::with_device_write_bypass(bus, |bus| {
            (
                bus.read_u32(addr).unwrap_or(0),
                bus.read_u32(addr + 4).unwrap_or(0) as u64,
                bus.read_u32(addr + 8).unwrap_or(0) as u64,
            )
        });
        Self {
            dw0,
            len: (dw0 >> 12) & 0xFFF,
            size: dw0 & 0xFFF,
            buf,
            next,
        }
    }

    fn dma_owned(&self) -> bool {
        self.dw0 & DESC_OWNER_BIT != 0
    }

    fn suc_eof(&self) -> bool {
        self.dw0 & DESC_SUC_EOF_BIT != 0
    }

    /// Return the descriptor to the CPU (owner bit cleared; an RX descriptor
    /// also stores the received length in bits [23:12]).
    fn write_back_owner(&self, bus: &mut dyn Bus, addr: u64, rx_len: Option<u32>) {
        let dw0 = match rx_len {
            Some(n) => (self.dw0 & !(DESC_OWNER_BIT | DESC_LEN_MASK)) | (n << 12),
            None => self.dw0 & !DESC_OWNER_BIT,
        };
        let _ = crate::with_device_write_bypass(bus, |bus| bus.write_u32(addr, dw0));
    }
}

/// Register kind within a channel's IN block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InReg {
    Conf0,
    Conf1,
    FifoStatus,
    Pop,
    Link,
    State,
    SucEofDesAddr,
    ErrEofDesAddr,
    Dscr,
    DscrBf0,
    DscrBf1,
    Pri,
    PeriSel,
}

/// Register kind within a channel's OUT block (`OUT_CONF1`-relative layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutReg {
    Conf1,
    FifoStatus,
    Push,
    Link,
    State,
    EofDesAddr,
    EofBfrDesAddr,
    Dscr,
    DscrBf0,
    DscrBf1,
    Pri,
    PeriSel,
    Conf0,
}

/// ESP32-C6 GDMA controller — three channels × {IN, OUT}.
#[derive(Debug)]
pub struct Esp32c6Gdma {
    channels: [Channel; NUM_CHANNELS],
    ahb_test: u32,
    misc_conf: u32,
    date: u32,
    /// Interrupt-matrix source id for IN channel 0 (`ETS_DMA_IN_CH0` = 66 on
    /// the C6). IN_CHn = base + n; OUT_CHn = base + NUM_CHANNELS + n.
    dma_in_ch0_source: u32,
}

impl Esp32c6Gdma {
    pub fn new(dma_in_ch0_source: u32) -> Self {
        Self {
            channels: [Channel::default(); NUM_CHANNELS],
            ahb_test: 0,
            misc_conf: 0,
            date: 0,
            dma_in_ch0_source,
        }
    }

    /// True when `PERI_SEL` names a real peripheral (not a Dummy value, not
    /// the unbound reset value). C6 TRM §3.4.2: 0=SPI2, 2=UHCI0, 3=I2S0,
    /// 6=AES, 7=SHA, 8=ADC, 9=PARLIO; 1/4/5/10..15 are Dummy.
    fn is_bound_peripheral(sel: u32) -> bool {
        matches!(sel & PERI_SEL_MASK, 0 | 2 | 3 | 6 | 7 | 8 | 9)
    }

    /// Reconstruct the full bus address from the 20-bit LINK field.
    fn full_desc_addr(link_addr: u32) -> u64 {
        u64::from(DESC_ADDR_PREFIX | (link_addr & LINK_ADDR_MASK))
    }

    /// Decode an offset inside channel `ch`'s IN block.
    fn in_reg_of(offset: u64, ch: usize) -> Option<InReg> {
        let base = IN_BLOCK_BASE + ch as u64 * CHANNEL_STRIDE;
        Some(match offset.checked_sub(base)? {
            0x00 => InReg::Conf0,
            0x04 => InReg::Conf1,
            0x08 => InReg::FifoStatus,
            0x0C => InReg::Pop,
            0x10 => InReg::Link,
            0x14 => InReg::State,
            0x18 => InReg::SucEofDesAddr,
            0x1C => InReg::ErrEofDesAddr,
            0x20 => InReg::Dscr,
            0x24 => InReg::DscrBf0,
            0x28 => InReg::DscrBf1,
            0x2C => InReg::Pri,
            0x30 => InReg::PeriSel,
            _ => return None,
        })
    }

    /// Decode an offset inside channel `ch`'s OUT register groups. The C6
    /// splits them across two bases (`OUT_CONF1_BASE` and `OUT_CONF0_BASE`);
    /// both are checked explicitly because `OUT_CONF0_CH0` (0x190) overlaps
    /// the sweep of `OUT_CONF1_CH1` (0x194).
    fn out_reg_of(offset: u64, ch: usize) -> Option<OutReg> {
        let b1 = OUT_CONF1_BASE + ch as u64 * CHANNEL_STRIDE;
        if offset >= b1 && offset < b1 + 0x30 {
            return Some(match offset - b1 {
                0x00 => OutReg::Conf1,
                0x04 => OutReg::FifoStatus,
                0x08 => OutReg::Push,
                0x0C => OutReg::Link,
                0x10 => OutReg::State,
                0x14 => OutReg::EofDesAddr,
                0x18 => OutReg::EofBfrDesAddr,
                0x1C => OutReg::Dscr,
                0x20 => OutReg::DscrBf0,
                0x24 => OutReg::DscrBf1,
                0x28 => OutReg::Pri,
                0x2C => OutReg::PeriSel,
                _ => return None,
            });
        }
        let b0 = OUT_CONF0_BASE + ch as u64 * CHANNEL_STRIDE;
        if offset == b0 {
            return Some(OutReg::Conf0);
        }
        None
    }

    /// Find `(channel, IN register)` for an IN-window offset.
    fn decode_in(offset: u64) -> Option<(usize, InReg)> {
        for ch in 0..NUM_CHANNELS {
            if let Some(reg) = Self::in_reg_of(offset, ch) {
                return Some((ch, reg));
            }
        }
        None
    }

    /// Find `(channel, OUT register)` for an OUT-window offset.
    fn decode_out(offset: u64) -> Option<(usize, OutReg)> {
        for ch in 0..NUM_CHANNELS {
            if let Some(reg) = Self::out_reg_of(offset, ch) {
                return Some((ch, reg));
            }
        }
        None
    }

    /// Decode a flat `{IN,OUT}_INT_{RAW,ST,ENA,CLR}` offset.
    fn decode_flat_int(offset: u64) -> Option<(bool, usize, u64)> {
        let (is_in, rel) = if (IN_INT_BASE..OUT_INT_BASE).contains(&offset) {
            (true, offset - IN_INT_BASE)
        } else if (OUT_INT_BASE..0x60).contains(&offset) {
            (false, offset - OUT_INT_BASE)
        } else {
            return None;
        };
        let ch = (rel / INT_CHANNEL_STRIDE) as usize;
        if ch >= NUM_CHANNELS {
            return None;
        }
        Some((is_in, ch, rel % INT_CHANNEL_STRIDE))
    }

    fn read_word(&self, offset: u64) -> u32 {
        if offset == AHB_TEST_OFFSET {
            return self.ahb_test;
        }
        if offset == MISC_CONF_OFFSET {
            return self.misc_conf;
        }
        if offset == DATE_OFFSET {
            return self.date;
        }
        if let Some((is_in, ch, blk)) = Self::decode_flat_int(offset) {
            let dir = if is_in {
                &self.channels[ch].rx
            } else {
                &self.channels[ch].tx
            };
            return match blk {
                INT_RAW => dir.int_raw,
                INT_ST => dir.int_raw & dir.int_ena,
                INT_ENA => dir.int_ena,
                INT_CLR => 0, // write-only
                _ => 0,
            };
        }
        if let Some((ch, reg)) = Self::decode_in(offset) {
            let c = &self.channels[ch];
            return match reg {
                InReg::Conf0 => c.rx.conf0,
                InReg::Conf1 => c.rx.conf1,
                InReg::FifoStatus => FIFO_EMPTY_BIT,
                InReg::Pop => 0, // write-only
                InReg::Link => {
                    (c.rx.link_addr & LINK_ADDR_MASK)
                        | if c.rx.link_auto_ret {
                            IN_LINK_AUTO_RET_BIT
                        } else {
                            0
                        }
                        | IN_LINK_PARK_BIT
                }
                InReg::State
                | InReg::ErrEofDesAddr
                | InReg::Dscr
                | InReg::DscrBf0
                | InReg::DscrBf1
                | InReg::Pri => 0,
                InReg::SucEofDesAddr => c.in_suc_eof_des_addr,
                InReg::PeriSel => c.rx.peri_sel & PERI_SEL_MASK,
            };
        }
        if let Some((ch, reg)) = Self::decode_out(offset) {
            let c = &self.channels[ch];
            return match reg {
                OutReg::Conf0 => c.tx.conf0,
                OutReg::Conf1 => c.tx.conf1,
                OutReg::FifoStatus => FIFO_EMPTY_BIT,
                OutReg::Push => 0, // write-only
                OutReg::Link => (c.tx.link_addr & LINK_ADDR_MASK) | OUT_LINK_PARK_BIT,
                OutReg::State
                | OutReg::EofBfrDesAddr
                | OutReg::Dscr
                | OutReg::DscrBf0
                | OutReg::DscrBf1
                | OutReg::Pri => 0,
                OutReg::EofDesAddr => c.out_eof_des_addr,
                OutReg::PeriSel => c.tx.peri_sel & PERI_SEL_MASK,
            };
        }
        crate::census_reg!("esp32c6.gdma:Esp32c6Gdma", offset, "read");
        0
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if offset == AHB_TEST_OFFSET {
            self.ahb_test = value;
            return;
        }
        if offset == MISC_CONF_OFFSET {
            self.misc_conf = value;
            return;
        }
        if offset == DATE_OFFSET {
            self.date = value;
            return;
        }
        if let Some((is_in, ch, blk)) = Self::decode_flat_int(offset) {
            match (is_in, blk) {
                (_, INT_RAW) => {} // RAW is not directly writable
                (true, INT_ENA) => self.channels[ch].rx.int_ena = value,
                (false, INT_ENA) => self.channels[ch].tx.int_ena = value,
                (true, INT_CLR) => self.channels[ch].rx.int_raw &= !value,
                (false, INT_CLR) => self.channels[ch].tx.int_raw &= !value,
                _ => {} // ST is RO
            }
            return;
        }
        if let Some((ch, reg)) = Self::decode_in(offset) {
            self.write_in_reg(ch, reg, value);
            return;
        }
        if let Some((ch, reg)) = Self::decode_out(offset) {
            self.write_out_reg(ch, reg, value);
            return;
        }
        crate::census_reg!("esp32c6.gdma:Esp32c6Gdma", offset, "write");
    }

    fn write_in_reg(&mut self, ch: usize, reg: InReg, value: u32) {
        match reg {
            InReg::Conf0 => self.channels[ch].rx.conf0 = value,
            InReg::Conf1 => self.channels[ch].rx.conf1 = value,
            InReg::FifoStatus
            | InReg::State
            | InReg::SucEofDesAddr
            | InReg::ErrEofDesAddr
            | InReg::Dscr
            | InReg::DscrBf0
            | InReg::DscrBf1
            | InReg::Pri => {}
            InReg::Pop => {} // no FIFO engine
            InReg::Link => {
                self.channels[ch].rx.link_addr = value & LINK_ADDR_MASK;
                self.channels[ch].rx.link_auto_ret = value & IN_LINK_AUTO_RET_BIT != 0;
                if value & (IN_LINK_START_BIT | IN_LINK_RESTART_BIT) != 0 {
                    self.kick_in(ch);
                }
                let _ = IN_LINK_STOP_BIT;
            }
            InReg::PeriSel => self.channels[ch].rx.peri_sel = value & PERI_SEL_MASK,
        }
    }

    fn write_out_reg(&mut self, ch: usize, reg: OutReg, value: u32) {
        match reg {
            OutReg::Conf0 => self.channels[ch].tx.conf0 = value,
            OutReg::Conf1 => self.channels[ch].tx.conf1 = value,
            OutReg::FifoStatus
            | OutReg::State
            | OutReg::EofDesAddr
            | OutReg::EofBfrDesAddr
            | OutReg::Dscr
            | OutReg::DscrBf0
            | OutReg::DscrBf1
            | OutReg::Pri => {}
            OutReg::Push => {} // no FIFO engine
            OutReg::Link => {
                self.channels[ch].tx.link_addr = value & LINK_ADDR_MASK;
                if value & (OUT_LINK_START_BIT | OUT_LINK_RESTART_BIT) != 0 {
                    self.kick_out(ch);
                }
                let _ = OUT_LINK_STOP_BIT;
            }
            OutReg::PeriSel => self.channels[ch].tx.peri_sel = value & PERI_SEL_MASK,
        }
    }

    fn kick_in(&mut self, ch: usize) {
        let mem_trans = self.channels[ch].rx.conf0 & MEM_TRANS_EN_BIT != 0;
        if mem_trans {
            self.channels[ch].in_started = true;
            if self.channels[ch].out_started {
                self.channels[ch].pending_m2m = true;
            }
        } else if Self::is_bound_peripheral(self.channels[ch].rx.peri_sel) {
            self.channels[ch].in_coupled_pending = true;
        } else {
            // Dummy/unbound: fallback auto-complete (zero bytes moved).
            self.channels[ch].rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
        }
    }

    fn kick_out(&mut self, ch: usize) {
        let mem_trans = self.channels[ch].rx.conf0 & MEM_TRANS_EN_BIT != 0;
        if mem_trans {
            self.channels[ch].out_started = true;
            if self.channels[ch].in_started {
                self.channels[ch].pending_m2m = true;
            }
        } else if Self::is_bound_peripheral(self.channels[ch].tx.peri_sel) {
            self.channels[ch].out_coupled_pending = true;
        } else {
            self.channels[ch].tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
        }
    }

    /// Walk an OUT (TX) descriptor chain and collect its bytes. Stops at
    /// `next == 0`, at a CPU-owned descriptor when `check_owner` is set, or at
    /// the hop bound. Returns the bytes and the last descriptor address.
    fn walk_out_chain(
        bus: &mut dyn Bus,
        desc_addr: u64,
        auto_wrback: bool,
        check_owner: bool,
    ) -> (Vec<u8>, u32) {
        let mut bytes = Vec::new();
        let mut addr = desc_addr;
        let mut last = 0u32;
        for _ in 0..MAX_DESC_CHAIN {
            if addr == 0 {
                break;
            }
            let d = Desc::read(bus, addr);
            if check_owner && !d.dma_owned() {
                break;
            }
            crate::with_device_write_bypass(bus, |bus| {
                for i in 0..d.len {
                    bytes.push(bus.read_u8(d.buf + u64::from(i)).unwrap_or(0));
                }
            });
            last = addr as u32;
            if auto_wrback {
                d.write_back_owner(bus, addr, None);
            }
            if d.next == 0 {
                break;
            }
            addr = d.next;
        }
        (bytes, last)
    }

    /// Write `bytes` into an IN (RX) descriptor chain. Each filled descriptor
    /// gets owner cleared and its length field set to the bytes written.
    /// Returns the last descriptor address touched.
    fn walk_in_chain(bus: &mut dyn Bus, desc_addr: u64, bytes: &[u8], check_owner: bool) -> u32 {
        let mut remaining = bytes;
        let mut addr = desc_addr;
        let mut last = 0u32;
        for _ in 0..MAX_DESC_CHAIN {
            if addr == 0 || remaining.is_empty() {
                break;
            }
            let d = Desc::read(bus, addr);
            if check_owner && !d.dma_owned() {
                break;
            }
            let to_write = remaining.len().min(d.size as usize);
            crate::with_device_write_bypass(bus, |bus| {
                for (i, &b) in remaining[..to_write].iter().enumerate() {
                    let _ = bus.write_u8(d.buf + i as u64, b);
                }
            });
            remaining = &remaining[to_write..];
            d.write_back_owner(bus, addr, Some(to_write as u32));
            last = addr as u32;
            if d.next == 0 || d.suc_eof() || remaining.is_empty() {
                break;
            }
            addr = d.next;
        }
        last
    }

    /// Serve the directions bound to the GP-SPI2 (`PERI_SEL` = 0): the C6
    /// GP-SPI2 is the C3's controller model, and the coupling is the one the
    /// C3 GDMA uses ([`crate::peripherals::esp_gdma`]).
    fn pump_spi2(&mut self, bus: &mut dyn Bus) {
        use crate::peripherals::esp_gdma::{
            fill_in, spi2_in_delivered, spi2_in_take, spi2_out_load, walk_out,
        };
        for ch in 0..NUM_CHANNELS {
            let c = self.channels[ch];
            if c.out_coupled_pending && c.tx.peri_sel & PERI_SEL_MASK == 0 {
                let w = walk_out(
                    bus,
                    Self::full_desc_addr(c.tx.link_addr) as u32,
                    c.tx.conf1 & CHECK_OWNER_BIT != 0,
                    c.tx.conf0 & OUT_AUTO_WRBACK_BIT != 0,
                    &DMA_RAM,
                );
                spi2_out_load(bus, &w.bytes);
                let c = &mut self.channels[ch];
                c.out_coupled_pending = false;
                c.out_eof_des_addr = w.last_desc;
                if w.descs > 0 {
                    c.tx.int_raw |= OUT_DONE_BIT;
                }
                if w.eof {
                    c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT;
                }
                if w.dscr_err {
                    c.tx.int_raw |= OUT_DSCR_ERR_BIT;
                }
            }
            if c.in_coupled_pending && c.rx.peri_sel & PERI_SEL_MASK == 0 {
                if let Some(rx) = spi2_in_take(bus) {
                    let f = fill_in(
                        bus,
                        Self::full_desc_addr(c.rx.link_addr) as u32,
                        &rx,
                        c.rx.conf1 & CHECK_OWNER_BIT != 0,
                        &DMA_RAM,
                    );
                    spi2_in_delivered(bus);
                    let c = &mut self.channels[ch];
                    c.in_coupled_pending = false;
                    c.in_suc_eof_des_addr = f.last_desc;
                    for (hit, bit) in [
                        (f.descs > 0, IN_DONE_BIT),
                        (f.suc_eof, IN_SUC_EOF_BIT),
                        (f.dscr_err, IN_DSCR_ERR_BIT),
                        (f.dscr_empty, IN_DSCR_EMPTY_BIT),
                    ] {
                        if hit {
                            c.rx.int_raw |= bit;
                        }
                    }
                }
            }
        }
    }

    /// Execute pending descriptor walks. Called from `tick_with_bus`.
    fn do_tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.pump_spi2(bus);
        for ch in 0..NUM_CHANNELS {
            if !self.channels[ch].pending_m2m {
                continue;
            }
            self.channels[ch].pending_m2m = false;
            self.channels[ch].in_started = false;
            self.channels[ch].out_started = false;

            let out_desc_addr = Self::full_desc_addr(self.channels[ch].tx.link_addr);
            let in_desc_addr = Self::full_desc_addr(self.channels[ch].rx.link_addr);
            let auto_wrback = self.channels[ch].tx.conf0 & OUT_AUTO_WRBACK_BIT != 0;
            let out_check_owner = self.channels[ch].tx.conf1 & CHECK_OWNER_BIT != 0;
            let in_check_owner = self.channels[ch].rx.conf1 & CHECK_OWNER_BIT != 0;

            let (bytes, out_last) =
                Self::walk_out_chain(bus, out_desc_addr, auto_wrback, out_check_owner);
            let in_last = if bytes.is_empty() {
                0
            } else {
                Self::walk_in_chain(bus, in_desc_addr, &bytes, in_check_owner)
            };

            self.channels[ch].out_eof_des_addr = out_last;
            self.channels[ch].in_suc_eof_des_addr = in_last;
            // Completion flags latch regardless of byte count (zero-length
            // transfers complete on silicon too).
            self.channels[ch].rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
            self.channels[ch].tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
        }
    }

    /// Asserted interrupt-matrix sources (raw & enable).
    fn asserted_sources_into(&self, out: &mut Vec<u32>) {
        for (n, c) in self.channels.iter().enumerate() {
            if c.rx.int_raw & c.rx.int_ena != 0 {
                out.push(self.dma_in_ch0_source + n as u32);
            }
            if c.tx.int_raw & c.tx.int_ena != 0 {
                out.push(self.dma_in_ch0_source + NUM_CHANNELS as u32 + n as u32);
            }
        }
    }
}

impl Peripheral for Esp32c6Gdma {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        Ok(((self.read_word(word_off) >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= u32::from(value) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut explicit_irqs = Vec::new();
        self.asserted_sources_into(&mut explicit_irqs);
        PeripheralTickResult {
            explicit_irqs: if explicit_irqs.is_empty() {
                None
            } else {
                Some(explicit_irqs)
            },
            ..PeripheralTickResult::default()
        }
    }

    /// True while an M2M walk is armed or a coupled direction is pending.
    fn needs_bus_tick(&self) -> bool {
        self.channels
            .iter()
            .any(|c| c.pending_m2m || c.in_coupled_pending || c.out_coupled_pending)
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.do_tick_with_bus(bus);
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        self.asserted_sources_into(out);
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
