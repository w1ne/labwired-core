// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// The family-specific register STATE lives in the `SpiRegs` enum: an STM32 SPI
// instance carries ONLY the STM32 registers, an nRF SPIM carries ONLY the
// Nordic registers — neither can hold the other's state. The shared transfer
// engine, attached-device routing and event-scheduler glue stay on `Spi`
// (genuinely shared behaviour), so the public API (`attach`, `set_loopback`,
// `as_any`) is unchanged. The chip-yaml `profile` selects the variant.

use crate::{Bus, SimResult};
use std::cell::Cell;
use std::str::FromStr;
use std::sync::Arc;

use crate::peripherals::pad_lines::PadLines;
use crate::peripherals::spi_waveform::{NarrationFit, SpiFraming, SpiNarrator};

/// Re-export the off-chip SPI contract and the opt-in sampling mode so every
/// existing `impl`, bound and intra-doc link at this path keeps resolving.
pub use crate::peripherals::device::{SpiDevice, SpiSampling};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpiRegisterLayout {
    #[default]
    Stm32,
    /// STM32 families with a TX/RX FIFO + CR2.DS data-size field
    /// (G0/L4/F7/G4/WB — RM0444 §35, RM0351 §40, RM0410 §35, RM0440 §39).
    /// Identical register layout to `Stm32`, but a **16-bit DR write at
    /// DS≤8 packs two frames** (RM0351 §40.4.9 data packing) — modelled so
    /// firmware that wrongly uses a 16-bit DR access at 8-bit data size
    /// mis-renders in the sim exactly as it does on silicon.
    Stm32Fifo,
    /// STM32H5/H7 "SPI v3" IP (RM0481 §41) — a different peripheral from the
    /// classic/FIFO map: 32-bit registers, split CFG1/CFG2 configuration,
    /// write-1-to-clear IFCR, CR2.TSIZE frame counting with SR.CTSIZE, and a
    /// CR1.CSTART-gated transfer engine. See [`Stm32H5SpiRegs`].
    Stm32H5,
    Nrf52Spim,
    /// Nordic **nRF54L-family SPIM** (nRF54L05/10/15, nRF54LM20A/B). Same
    /// EasyDMA engine as [`SpiRegisterLayout::Nrf52Spim`] on relocated
    /// offsets, plus hardware D/C (`PSEL.DCX` + `DCXCNT`) and a hardware chip
    /// select. See [`NrfSpimMap`] for the moved-register table and for why this
    /// is one engine with two maps rather than two models.
    Nrf54lSpim,
    /// NXP Kinetis **DSPI** (KW41Z `SPI0/SPI1`) — FIFO master with MCR / TCR /
    /// CTAR / SR / PUSHR / POPR. A frame is transmitted by writing PUSHR (the
    /// low 16 bits are the data, the high bits select PCS / CONT / EOQ); the
    /// `fsl_dspi` blocking path polls SR.TFFF before the push and SR.TCF after.
    /// See [`KinetisDspiRegs`].
    KinetisDspi,
    /// Silicon Labs EFR32/EFM32 Series-2 **USART in synchronous mode** — the
    /// same block as the console UART, with `CTRL.SYNC` set and
    /// `CMD.MASTEREN` issued. There is no separate SPI peripheral on this
    /// family; a frame is a `TXDATA` write answered on `RXDATA`. See
    /// [`Efr32s2SpiRegs`].
    Efr32s2Usart,
}

impl FromStr for SpiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32" | "stm32f1" | "stm32f4" | "stm32v2" => Ok(Self::Stm32),
            "stm32_fifo" | "stm32l4" | "stm32f7" | "stm32g4" | "stm32g0" => Ok(Self::Stm32Fifo),
            // H5 carries the H7-lineage "SPI v3" IP, not the L4/F7 FIFO map.
            "stm32h5" => Ok(Self::Stm32H5),
            "nrf52" | "nrf52_spim" | "nrf_spim" | "nordic" => Ok(Self::Nrf52Spim),
            "nrf54l" | "nrf54l_spim" | "nrf54lm20a" | "nrf54l15" => Ok(Self::Nrf54lSpim),
            "kinetis" | "dspi" | "kinetis_dspi" | "nxp_dspi" | "kw41z" => Ok(Self::KinetisDspi),
            "efr32s2" | "efr32" | "efm32" | "gecko" | "efr32s2_usart" => Ok(Self::Efr32s2Usart),
            _ => Err(format!(
                "unsupported SPI register layout '{}'; supported: stm32, stm32_fifo, stm32h5, nrf52, nrf54l, kinetis",
                value
            )),
        }
    }
}

/// Event token for the SPI bit engine's next-wire-transition event (STM32).
const SPI_DONE_TOKEN: u32 = 0;
/// Event token for nRF52 SPIM EasyDMA completion (delay-0 scheduler path).
const SPI_NRF52_EASYDMA_TOKEN: u32 = 1;
/// Event token delivering an interrupt a world `gpio_net` edge raised.
const SPI_NET_IRQ_TOKEN: u32 = 2;
/// Scheduler token: raise the STM32 SPI's due DMA requests (CR2.TXDMAEN /
/// RXDMAEN). See [`Spi::stm32_take_dma_signals`]. (3 is `spi_nrf_net`'s.)
const SPI_DMA_TOKEN: u32 = 4;
/// H5 "SPI v3" level IRQ re-check (SR & IER). Separate from the bit-engine
/// done token because this IP has no frame timeline of its own.
const SPI_H5_IRQ_TOKEN: u32 = 8;
/// Cycles between re-checks of the DMA request lines while requests are
/// being answered — long enough for the routed DMA channel to move the item
/// (its event fires the cycle after the request), short against a frame.
const SPI_DMA_RECHECK: u64 = 2;

/// High bit marks an H5 wire-narration wakeup. The low 31 bits carry the arm
/// sequence, so a stale wakeup from a superseded arm is recognisable — and
/// because the flag is always set, an H5 token can never collide with
/// [`SPI_DONE_TOKEN`] (0) or [`SPI_NRF52_EASYDMA_TOKEN`] (1) no matter how far
/// the sequence wraps.
const SPI_H5_WIRE_TOKEN_FLAG: u32 = 0x8000_0000;

/// Same idea for the EFR32 burst: a flag bit that cannot collide with the
/// small tokens above.
const SPI_EFR32_WIRE_TOKEN: u32 = 0x4000_0000;

const fn h5_wire_token(seq: u32) -> u32 {
    SPI_H5_WIRE_TOKEN_FLAG | (seq & 0x7FFF_FFFF)
}

// ── STM32 SPI wire (bit-level engine) ────────────────────────────────────────
//
// The classic/FIFO STM32 SPI no longer completes a DR write instantly: a bit
// engine clocks the frame on the wire over simulated cycles, mirroring the
// ESP32-C3 I²C bit-level engine (core#507). SCK timing derives from CR1
// BR[2:0] against the peripheral clock (this simulator's cycle base models
// PCLK, the same convention every other STM32 peripheral here uses):
// f_SCK = f_PCLK / 2^(BR+1), so one SCK half-period is 2^BR peripheral-clock
// cycles and a frame is `bits × 2^(BR+1)` cycles. CPOL sets the idle level,
// CPHA selects the sample edge (data is driven for the whole bit period;
// sample = leading edge at CPHA=0, trailing edge at CPHA=1), LSBFIRST picks
// the shift direction, and the frame size comes from CR2.DS on FIFO ports
// (L4/F7/G4) or CR1.DFF on classic ports (F1/F4) — datasheet reset values
// apply when firmware never programs them.
//
// Slaves stay byte-level BY DEFAULT ([`SpiDevice`], behind the TracingSpiDevice
// choke point): the engine consults them once per frame, at the frame boundary
// where the frame starts clocking, and the byte the device answers is what
// MISO carries bit-by-bit during that SAME frame — full duplex, like real
// silicon exchanging shift registers. (Frames wider than 8 bits still
// exchange one byte with the byte-level device; the wire carries the full
// programmed frame — an honest limit of the byte-level device contract.)
//
// A device may OPT IN to edge-accurate sampling by overriding
// [`SpiDevice::sampling`] with [`SpiSampling::Edge`], declaring the CPOL/CPHA
// its own silicon is strapped for. The engine then latches MOSI into it, and
// clocks MISO out of it, on the physical SCK edges that mode selects, so a
// master/slave mode mismatch corrupts data instead of being invisible. Nothing
// on the default path changes: the opt-in is a single `Option` test per frame
// (`edge_slave`, resolved once at attach time) and zero extra work per bit —
// pinned byte for byte by `tests::spi_byte_level_golden`.

/// SPI signal roles on the wire, used by the GPIO AF pad routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiSignal {
    Sck,
    Mosi,
    Miso,
    /// Hardware slave select. Only a part whose AF table routes it (the G0,
    /// F4, L4, G4, WB and L0 tables) puts it on a pad, and only a controller
    /// on a world `gpio_net` drives or reads it (see `spi_net`). On the nRF52
    /// SPIM3 this line is the hardware `CSN` (see `spi_nrf_net`).
    Nss,
}

/// Live SCK/MOSI/MISO levels of one STM32 SPI controller's wire.
///
/// A thin, named face over [`PadLines`], the ONE pad-publication mechanism
/// (see `peripherals::pad_lines`). The controller bit engine is the only
/// writer; the STM32 `GpioPort` reads it for pads whose MODER/AFR (or F1
/// CRL/CRH CNF) route this SPI's alternate function, so `read_gpio_pad` — and
/// the in-engine logic analyzer sampling through it — observes the real
/// waveform on the routed pads. With push-mode capture armed on a routed pad,
/// [`set`](Self::set) additionally reports each line transition into the
/// shared logic tap at drive time.
#[derive(Debug)]
pub struct SpiLineLevels {
    lines: std::sync::Arc<PadLines>,
}

/// Line order for [`SpiLineLevels`]. The accessors index [`PadLines`] by
/// `SpiSignal as usize`, so this order IS the enum's discriminant order —
/// pinned by `spi_line_order_matches_signal_discriminants`.
const SPI_LINES: &[&str] = &["SCK", "MOSI", "MISO", "NSS"];

/// Line order of an nRF52 SPIM's wire: [`SPI_LINES`] with the select spelled
/// `CSN`, as Nordic does, plus SPIM3's data/command line. Only SPIM3 drives
/// the last two, and only on a world `gpio_net` (`spi_nrf_net`).
const NRF_SPIM_LINES: &[&str] = &["SCK", "MOSI", "MISO", "CSN", "DCX"];

/// [`NRF_SPIM_LINES`] index of the SPIM3 D/CX line.
pub(crate) const NRF_SPIM_LINE_DCX: usize = 4;

impl SpiLineLevels {
    fn new(sck_idle: bool) -> Self {
        Self {
            // MOSI/MISO idle low; SCK idles at CPOL.
            // NSS idles high (deselected).
            lines: std::sync::Arc::new(PadLines::new(SPI_LINES, &[sck_idle, false, false, true])),
        }
    }

    /// An nRF52 SPIM's wire ([`NRF_SPIM_LINES`]): CSN and DCX idle high.
    fn new_nrf(sck_idle: bool) -> Self {
        Self {
            lines: std::sync::Arc::new(PadLines::new(
                NRF_SPIM_LINES,
                &[sck_idle, false, false, true, true],
            )),
        }
    }

    /// The underlying pad lines, for a GPIO port to route pads to.
    ///
    /// A port routes a pad to a `(PadLines, line)` pair and knows nothing about
    /// which peripheral owns it — that is what lets one routing mechanism serve
    /// SPI, I²C and whatever publishes pads next.
    pub(crate) fn pad_lines(&self) -> &std::sync::Arc<PadLines> {
        &self.lines
    }

    pub fn sck(&self) -> bool {
        self.lines.level(SpiSignal::Sck as usize)
    }

    pub fn mosi(&self) -> bool {
        self.lines.level(SpiSignal::Mosi as usize)
    }

    pub fn miso(&self) -> bool {
        self.lines.level(SpiSignal::Miso as usize)
    }

    pub fn level(&self, signal: SpiSignal) -> bool {
        self.lines.level(signal as usize)
    }

    fn set(&self, sck: bool, mosi: bool, miso: bool) {
        // Every line but the three data/clock ones keeps its level (NSS on
        // an STM32; CSN and DCX on an nRF SPIM). No allocation: this runs
        // per half bit on the STM32 bit engine.
        let n = self.lines.names().len();
        let mut levels = [sck, mosi, miso, false, false];
        for (line, level) in levels.iter_mut().enumerate().take(n).skip(3) {
            *level = self.lines.level(line);
        }
        self.lines.set(&levels[..n]);
    }
}

/// Wire timing snapshot, derived from the live CR1/CR2 registers at frame
/// start (datasheet reset values apply when firmware never programs them —
/// no invented constants).
#[derive(Debug, Clone, Copy, serde::Serialize)]
struct FrameTiming {
    /// SCK half-period in peripheral-clock cycles = `2^BR` (CR1 BR[5:3]).
    half_ticks: u32,
    /// Frame size in bits: CR2.DS+1 on FIFO ports, CR1.DFF ? 16 : 8 on
    /// classic ports.
    bits: u8,
    /// CR1.CPOL — SCK idle level.
    cpol: bool,
    /// CR1.CPHA — sample on leading (0) or trailing (1) edge.
    cpha: bool,
    /// CR1.LSBFIRST — shift direction.
    lsb_first: bool,
}

impl FrameTiming {
    /// The clock-mode view the shared edge model consumes.
    fn wire(&self) -> WireMode {
        WireMode {
            bits: self.bits,
            cpol: self.cpol,
            cpha: self.cpha,
            lsb_first: self.lsb_first,
        }
    }
}

/// The clock mode + frame shape one edge-accurate exchange needs: everything
/// [`edge_slave_capture`] and [`edge_miso_wire`] read, and nothing else.
///
/// Deliberately NOT the STM32-specific [`FrameTiming`]: the ESP32-C3 GP-SPI
/// controller has no SCK divider or half-period to speak of and still has a
/// clock mode, so both controllers hand the SAME two functions this, and there
/// is exactly one implementation of what an edge means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WireMode {
    /// Frame size in bits.
    pub bits: u8,
    /// SCK idle level.
    pub cpol: bool,
    /// Sample on the leading (false) or trailing (true) edge.
    pub cpha: bool,
    /// Shift direction.
    pub lsb_first: bool,
}

/// The frame currently shifting on the wire. Every bit period is two counted
/// half-periods; the (SCK, MOSI, MISO) levels are a pure function of this
/// state (see [`Spi::stm32_frame_levels`]).
#[derive(Debug, Clone, Copy, serde::Serialize)]
struct ActiveFrame {
    t: FrameTiming,
    /// The full frame value clocked out on MOSI.
    mosi: u16,
    /// The full frame value clocked in on MISO (the slave's byte answer).
    miso: u16,
    /// Edge-sampled slave only: the level MISO carries in each half-period of
    /// this frame, bit `h` = half-period `h` (`h = 2*bit_idx + second_half`),
    /// produced by [`Spi::edge_miso_wire`]. `None` — the default byte-level
    /// path — renders MISO from `miso` with the MASTER's phase (one value held
    /// for the whole bit period), exactly as before.
    miso_halves: Option<u32>,
    /// Bit currently on the wire, 0-based in shift order.
    bit_idx: u8,
    /// Second half of the current bit period.
    second_half: bool,
    /// Peripheral-clock cycles left in the current half-period.
    ticks_left: u32,
    /// Not a frame: the NSS-high pulse a FIFO-type master inserts between two
    /// back-to-back frames with CR2.NSSP=1 (RM0444 §35.5.5 "NSS pulse mode":
    /// NSS is pulsed high for one SCK period between continuous frames). SCK
    /// rests at CPOL and the data lines hold; `ticks_left` counts the whole
    /// pulse, after which the queued frame starts.
    nss_pulse: bool,
}

// ── Edge-accurate slave sampling (shared by every controller with a clock mode)
//
// ONE implementation of what an SCK edge means. The STM32 bit engine below and
// the ESP32-C3 GP-SPI controller both call these; a second copy would be a
// second answer to "which edge does a mode-2 slave sample on?".

/// Walk the SCK edges one frame puts on the wire, in order, calling
/// `visit(half, new_level)`.
///
/// `half` is the half-period the edge STARTS (`h = 2*bit + second_half`),
/// so the line levels in force just BEFORE the edge are those of half
/// `h-1`; `half == 2*bits` is the trailing edge back to the CPOL idle level
/// after the last half-period. SCK during half `h` is
/// `cpol ^ cpha ^ (h odd)` — the same function [`Spi::stm32_frame_levels`]
/// renders — so at CPHA=0 the frame opens with no edge (the first half
/// already sits at the idle level) and closes with one, and at CPHA=1 the
/// reverse. Either way a frame carries exactly `2 × bits` edges.
pub(crate) fn for_each_edge(t: &WireMode, mut visit: impl FnMut(u32, bool)) {
    let halves = 2 * u32::from(t.bits);
    for h in 0..halves {
        if h == 0 && !t.cpha {
            continue; // level(0) == CPOL: no transition out of idle
        }
        visit(h, t.cpol ^ t.cpha ^ (h & 1 != 0));
    }
    if !t.cpha {
        visit(halves, t.cpol);
    }
}

/// Bit `idx` of a frame word in wire order (MSB first unless LSBFIRST).
pub(crate) fn frame_bit(t: &WireMode, word: u16, idx: u8) -> bool {
    let shift = if t.lsb_first { idx } else { t.bits - 1 - idx };
    (word >> shift) & 1 != 0
}

/// The byte an edge-sampled slave actually latches off MOSI this frame.
///
/// The slave samples on the physical edge ITS mode selects — rising iff
/// `cpol == cpha` (mode 0/3 sample rising, mode 1/2 falling) — and takes
/// the level the line carried just BEFORE that edge. That pre-edge rule is
/// the deterministic reading of a coincident edge: the master changes MOSI
/// at bit boundaries, so a slave whose sample edge lands on a boundary
/// (a CPHA mismatch) is sampling into the master's transition, which on
/// silicon resolves through the driver's propagation delay to the OLD bit.
/// `prev_mosi` is the level the pad already carried when the frame opened,
/// which is what such a slave latches first.
pub(crate) fn edge_slave_capture(
    t: &WireMode,
    mosi: u16,
    prev_mosi: bool,
    s_cpol: bool,
    s_cpha: bool,
) -> u16 {
    let sample_rising = s_cpol == s_cpha;
    let mut rx = 0u16;
    let mut n: u8 = 0;
    for_each_edge(t, |h, level| {
        if level != sample_rising || n >= t.bits {
            return;
        }
        let before = if h == 0 {
            prev_mosi
        } else {
            frame_bit(t, mosi, ((h - 1) / 2) as u8)
        };
        if before {
            rx |= 1 << if t.lsb_first { n } else { t.bits - 1 - n };
        }
        n += 1;
    });
    rx
}

/// Clock the slave's answer out on MISO at the slave's shift edges and
/// latch it back at the master's sample edges.
///
/// Returns `(miso half-period map, word the master captured)`. A CPHA=0
/// slave presents its first bit as soon as the frame opens (real parts
/// drive it at CS↓); a CPHA=1 slave presents nothing until its first
/// leading edge, so a master that samples on that same edge latches
/// whatever the pad already carried — the classic one-bit shift a CPHA
/// mismatch produces on real hardware. The master, like the slave, takes
/// the pre-edge level.
pub(crate) fn edge_miso_wire(
    t: &WireMode,
    resp: u16,
    prev_miso: bool,
    s_cpol: bool,
    s_cpha: bool,
) -> (u32, u16) {
    let slave_sample_rising = s_cpol == s_cpha;
    let master_sample_rising = t.cpol == t.cpha;
    let halves = 2 * u32::from(t.bits);
    // Slave output register: CPHA=0 presents bit 0 immediately, CPHA=1
    // only from its first shift (leading) edge.
    let mut out_idx: i32 = if s_cpha { -1 } else { 0 };
    let mut level = if s_cpha {
        prev_miso
    } else {
        frame_bit(t, resp, 0)
    };
    let mut map = 0u32;
    let mut painted = 0u32;
    let mut rx = 0u16;
    let mut n: u8 = 0;
    for_each_edge(t, |h, edge_level| {
        while painted < h {
            map |= u32::from(level) << painted;
            painted += 1;
        }
        if edge_level == master_sample_rising && n < t.bits {
            if level {
                rx |= 1 << if t.lsb_first { n } else { t.bits - 1 - n };
            }
            n += 1;
        }
        if edge_level != slave_sample_rising {
            out_idx += 1;
            if out_idx >= 0 && (out_idx as u32) < u32::from(t.bits) {
                level = frame_bit(t, resp, out_idx as u8);
            }
        }
    });
    while painted < halves {
        map |= u32::from(level) << painted;
        painted += 1;
    }
    (map, rx)
}

/// STM32 SPI register file (F1/F4/L0 classic and L4/F7/G4 FIFO share this map;
/// `fifo` selects the FIFO DS/data-packing behaviour). H5/H7 use the separate
/// "SPI v3" map in [`Stm32H5SpiRegs`].
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Stm32SpiRegs {
    fifo: bool,
    cr1: u16,
    cr2: u16,
    sr: u16,
    dr: u16,
    crcpr: u16,
    rxcrcr: u16,
    txcrcr: u16,
    i2scfgr: u16,
    i2spr: u16,
}

impl Stm32SpiRegs {
    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.sr,
            // DR read returns the RX FIFO contents (`self.dr`), which is
            // distinct from what was last written. Real silicon has separate
            // TX and RX paths; we model that with `dr` for RX.
            0x0C => self.dr,
            0x10 => self.crcpr,
            0x14 => self.rxcrcr,
            0x18 => self.txcrcr,
            0x1C => self.i2scfgr,
            0x20 => self.i2spr,
            _ => {
                crate::census_reg!("spi:Stm32SpiRegs", offset, "read");
                0
            }
        }
    }
}

/// Capacity of the FIFO-type STM32 SPI's RX and TX FIFOs, in bytes: "two
/// 32-bit embedded Rx and Tx FIFOs" (RM0444 §35.5.9, RM0351 §40.4.9).
const FIFO_BYTES: u8 = 4;

// FIFO-type SPI_CR2 / SPI_SR bits (RM0444 §35.7.2, §35.7.3; same positions
// in RM0351/RM0440/RM0410 and the G071/L476/G474/WB55 SVDs).
/// CR2.NSSP — NSS pulse between frames (master, CPHA=0).
const FIFO_CR2_NSSP: u16 = 1 << 3;
/// CR2.FRXTH — RXNE threshold: 0 = 16 bit (2 bytes), 1 = 8 bit (1 byte).
const FIFO_CR2_FRXTH: u16 = 1 << 12;
/// CR2.LDMA_RX / LDMA_TX — the last DMA transfer of an odd-length packed
/// transfer moves ONE frame, not two (RM0444 §35.5.10, "Data packing with
/// DMA").
const FIFO_CR2_LDMA_RX: u16 = 1 << 13;
const FIFO_CR2_LDMA_TX: u16 = 1 << 14;
/// CR2.RXDMAEN / TXDMAEN (bits 0/1 on the classic and FIFO ports alike).
const CR2_RXDMAEN: u16 = 1 << 0;
const CR2_TXDMAEN: u16 = 1 << 1;
/// SR.FRLVL[1:0] (bits 10:9) and SR.FTLVL[1:0] (bits 12:11).
const FIFO_SR_FRLVL_SHIFT: u16 = 9;
const FIFO_SR_FTLVL_SHIFT: u16 = 11;
const FIFO_SR_LEVELS: u16 = (0b11 << FIFO_SR_FRLVL_SHIFT) | (0b11 << FIFO_SR_FTLVL_SHIFT);

/// The FIFO-type SPI's 32-bit RX FIFO, as bytes. A frame of up to 8 bits
/// takes one byte, a 9..16-bit frame two (low byte first), which is what
/// makes the 8-bit / 16-bit DR read sizes pop one or two bytes (RM0444
/// §35.5.9 "Data packing").
///
/// `Copy` so it can live in a `Cell`: `Peripheral::read` is `&self`, and a
/// DR read pops.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
struct RxFifo {
    buf: [u8; FIFO_BYTES as usize],
    len: u8,
}

impl RxFifo {
    /// Push one frame (`bytes` = 1 or 2). `false`, and nothing stored, when
    /// it does not fit — the overrun case.
    fn push_frame(&mut self, word: u16, bytes: u8) -> bool {
        if self.len + bytes > FIFO_BYTES {
            return false;
        }
        for i in 0..bytes {
            self.buf[usize::from(self.len)] = (word >> (8 * i)) as u8;
            self.len += 1;
        }
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.buf[0];
        self.buf.copy_within(1.., 0);
        self.len -= 1;
        Some(b)
    }
}

/// SR.FRLVL / SR.FTLVL encoding of a byte level: 00 empty, 01 1/4, 10 1/2,
/// 11 full (RM0444 §35.7.3). Three bytes reads as full, as the field has no
/// 3/4 code.
fn fifo_level_code(bytes: u8) -> u16 {
    u16::from(bytes.min(3))
}

/// STM32H5/H7 "SPI v3" register file (RM0481 §41) — H5-only state.
///
/// Register map (RM0481 / CMSIS stm32h563xx.h):
///   0x00 CR1, 0x04 CR2, 0x08 CFG1, 0x0C CFG2, 0x10 IER, 0x14 SR,
///   0x18 IFCR (write-only, reads 0), 0x20 TXDR (write-only),
///   0x30 RXDR (read-only), 0x40 CRCPOLY, 0x44 TXCRC, 0x48 RXCRC,
///   0x4C UDRDR, 0x50 I2SCFGR.
///
/// Reset values, write masks and the mode-fault/SPE-lock machinery are pinned
/// by silicon capture 2026-06-11 (NUCLEO-H563ZI), probed over SWD.
///
/// ── Known divergence from the bench capture ────────────────────────────────
/// The bench part had no SPI kernel clock configured, so real frames never
/// shifted: TXDR writes set TXTF but CTSIZE never moved. The sim is always
/// clocked, so with SPE+CSTART in master mode each TXDR write transmits one
/// frame and decrements CTSIZE (same class of divergence as the RNG
/// kernel-clock note in the chip yaml). RX is not modelled yet: the engine is
/// TX-only and RXDR always reads 0.
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Stm32H5SpiRegs {
    cr1: u32,
    cr2: u32,
    cfg1: u32,
    cfg2: u32,
    ier: u32,
    /// SR flag bits [15:0]; the CTSIZE field [31:16] lives in `ctsize`.
    sr: u32,
    /// SR.CTSIZE — remaining-frame count, loaded from CR2.TSIZE at SPE set.
    ctsize: u32,
    /// RXDR — the frame captured on MISO by the most recent transfer.
    rxdr: u32,
    crcpoly: u32,
    txcrc: u32,
    rxcrc: u32,
    udrdr: u32,
    i2scfgr: u32,
}

// ── STM32H5 SPI bit positions (RM0481 §41.4) ────────────────────────────────
/// CR1: peripheral enable.
const H5_CR1_SPE: u32 = 1 << 0;
/// CR1: master transfer start; HW-cleared when CTSIZE reaches 0.
const H5_CR1_CSTART: u32 = 1 << 9;
/// CR1: internal SS level when CFG2.SSM=1.
const H5_CR1_SSI: u32 = 1 << 12;
/// CR1 writable bits: SPE(0), MASRX(8), CSTART(9), HDDIR(11), SSI(12),
/// CRC33_17(13), RCRCINI(14), TCRCINI(15), IOLOCK(16). CSUSP(10) is a
/// write-only strobe and reads 0.
const H5_CR1_WRITABLE: u32 = 0x0001_FB01;

/// SR: TX-packet space available — always set (sim TX path is bottomless).
const H5_SR_RXP: u32 = 1 << 0;
const H5_SR_TXP: u32 = 1 << 1;
/// SR: duplex packet available (both directions can move a packet). With a
/// bottomless TX path this is live-RXP.
const H5_SR_DXP: u32 = 1 << 2;
/// SR: end of transfer (CTSIZE reached 0).
const H5_SR_EOT: u32 = 1 << 3;
/// SR: transmission of TxFIFO filled.
const H5_SR_TXTF: u32 = 1 << 4;
/// SR: mode fault.
const H5_SR_MODF: u32 = 1 << 9;
/// SR: transmission complete.
const H5_SR_TXC: u32 = 1 << 12;
/// SR reset value = TXP|TXC — silicon capture 2026-06-11 (NUCLEO-H563ZI).
const H5_SR_RESET: u32 = H5_SR_TXP | H5_SR_TXC;

/// CFG1 reserved bits, read as 0. Derived from the silicon round-trip triple
/// 0x70000007 / 0x00080008 / 0x5555AAAA→0x505582AA — capture 2026-06-11
/// (NUCLEO-H563ZI).
const H5_CFG1_RESERVED: u32 = 0x0500_2800;
/// CFG1 reset = MBR /8, CRCSIZE 8-bit, DSIZE 8-bit — silicon capture
/// 2026-06-11 (NUCLEO-H563ZI).
const H5_CFG1_RESET: u32 = 0x0007_0007;

/// CFG2: master mode select.
const H5_CFG2_MASTER: u32 = 1 << 22;
/// CFG2: software SS management.
const H5_CFG2_SSM: u32 = 1 << 26;
/// CFG1: Rx DMA stream enable (RM0456 §68.8.3).
const H5_CFG1_RXDMAEN: u32 = 1 << 14;
/// CFG1: Tx DMA stream enable.
const H5_CFG1_TXDMAEN: u32 = 1 << 15;

/// IER writable bits [10:0] (RXPIE..TSERFIE).
const H5_IER_WRITABLE: u32 = 0x0000_07FF;

/// IFCR write-1-to-clear mask: EOTC(3), TXTFC(4), UDRC(5), OVRC(6), CRCEC(7),
/// TIFREC(8), MODFC(9), SUSPC(11).
const H5_IFCR_W1C: u32 = 0x0000_0BF8;

/// CRCPOLY reset (CRC-8 x^8+x^2+x+1) — silicon capture 2026-06-11
/// (NUCLEO-H563ZI).
const H5_CRCPOLY_RESET: u32 = 0x0000_0107;

// ── H5 wire-narration fields (SVD-derived, configs/peripherals/stm32h563/
// spi1.yaml — the same schema the debugger inspects) ─────────────────────────
/// `SPI_CFG1.DSIZE [4:0]` — frame width minus one.
const H5_CFG1_DSIZE: u32 = 0x0000_001F;
/// `SPI_CFG1.MBR [30:28]` — master baud rate prescaler setting.
const H5_CFG1_MBR: u32 = 0x7000_0000;
const H5_CFG1_MBR_SHIFT: u32 = 28;
/// `SPI_CFG2.LSBFRST [23]` — shift direction (1 = LSB first).
const H5_CFG2_LSBFRST: u32 = 1 << 23;
/// `SPI_CFG2.CPHA [24]` — sample on leading (0) or trailing (1) edge.
const H5_CFG2_CPHA: u32 = 1 << 24;
/// `SPI_CFG2.CPOL [25]` — SCK idle level.
const H5_CFG2_CPOL: u32 = 1 << 25;

/// Frames held before an H5 burst is published compressed rather than buffered
/// further. Mirrors `Rp2040Spi`'s `WIRE_BURST_CAP` and exists for the same
/// reason: an unbounded buffer would grow with a display flush that no analyzer
/// window could show anyway.
const H5_WIRE_BURST_CAP: usize = 256;

impl Stm32H5SpiRegs {
    fn reset() -> Self {
        Self {
            cfg1: H5_CFG1_RESET,
            sr: H5_SR_RESET,
            crcpoly: H5_CRCPOLY_RESET,
            ..Default::default()
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.cfg1,
            0x0C => self.cfg2,
            0x10 => self.ier,
            // SR[31:16] = CTSIZE remaining-frame count, flags below.
            0x14 => (self.ctsize << 16) | self.sr,
            // IFCR (0x18) and TXDR (0x20) are write-only and read 0.
            0x18 | 0x20 => 0,
            // RXDR: the captured MISO frame.
            0x30 => self.rxdr,
            0x40 => self.crcpoly,
            0x44 => self.txcrc,
            0x48 => self.rxcrc,
            0x4C => self.udrdr,
            0x50 => self.i2scfgr,
            _ => {
                crate::census_reg!("spi:Stm32H5SpiRegs", offset, "read");
                0
            }
        }
    }
}

/// Nordic nRF52 SPIM (EasyDMA) register file — Nordic-only state.
///
/// Register offsets follow nRF52840 PS rev 1.7 §6.30 (SPIM).
///
/// TASKS:
///   0x010  TASKS_START  — write 1 arms EasyDMA; handled via needs_bus_tick/tick_with_bus
///   0x014  TASKS_STOP   — write 1 requests a graceful stop
///
/// EVENTS:
///   0x104  EVENTS_STOPPED  — peripheral stopped
///   0x110  EVENTS_ENDRX    — last byte clocked into RXD buffer (HW-set only)
///   0x118  EVENTS_END      — all RXD+TXD transfers complete (HW-set only)
///   0x120  EVENTS_ENDTX    — last byte clocked out of TXD buffer (HW-set only)
///
/// EVENTS write-semantics (silicon-verified for TIMER/RTC, applied uniformly):
///   SW writes of 1 are ignored — only HW sets EVENTS registers.
///   SW writes of 0 clear the event.
///
/// CONFIG:
///   0x554  CONFIG  — ORDER (bit 0), CPHA (bit 1), CPOL (bit 2)
///
/// EasyDMA:
///   0x534  RXD.PTR     — base address for received bytes
///   0x538  RXD.MAXCNT  — max bytes to receive
///   0x53C  RXD.AMOUNT  — bytes actually received (HW-updated, PS §6.30.4D0)
///   0x544  TXD.PTR     — base address for bytes to transmit
///   0x548  TXD.MAXCNT  — number of bytes to transmit
///   0x54C  TXD.AMOUNT  — bytes actually transmitted (HW-updated, PS §6.30.4D8)
///   0x5C0  ORC         — over-read character (sent when TXD exhausted but RXD still running)
/// Which generation's register offsets this instance answers on.
///
/// The nRF54L SPIM is the SAME transfer engine as the nRF52 SPIM — EasyDMA,
/// `ENABLE` = 7 to select the SPIM personality on a shared serial-instance
/// window, and a `CONFIG` at 0x554 whose ORDER/CPHA/CPOL bits sit in the same
/// places. What moved is the address of nearly everything else, exactly as it
/// moved for this family's UARTE and TWIM:
///
///   register           nRF52    nRF54L    note
///   TASKS_START        0x010    0x000
///   EVENTS_END         0x118    0x108
///   RX complete        0x110    0x14C     EVENTS_DMA.RX.END
///   TX complete        0x120    0x168     EVENTS_DMA.TX.END
///   PSEL.SCK           0x508    0x600
///   RX descriptor      0x534    0x704     DMA.RX.PTR
///   TX descriptor      0x544    0x73C     DMA.TX.PTR
///   bit rate           0x524    0x52C     FREQUENCY enum -> PRESCALER divisor
///
/// Source: Nordic MDK `nrf54lm20a_application.svd`, peripheral GLOBAL_SPIM00.
///
/// This is modelled as one engine with two offset maps rather than two models
/// because the BEHAVIOUR is not what changed. The UARTE and TWIM on this family
/// did need their own models — their task/event *semantics* diverged — but a
/// SPIM transfer here is byte-for-byte the nRF52 sequence at new addresses, and
/// a second copy of the EasyDMA engine would be a second place for it to rot.
///
/// ⚠️ `ENABLE` (0x500), `CONFIG` (0x554) and `ORC` (0x5C0) coincide across both
/// generations. That is the same coincidence that disguised the UARTE
/// incompatibility on this family: a wrong map still accepts the enable and the
/// mode write and looks alive, then never sees the DMA start task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
enum NrfSpimMap {
    /// nRF52/nRF5340 SPIM offsets.
    #[default]
    Nrf52,
    /// nRF54L-family SPIM offsets (nRF54L05/10/15, nRF54LM20A/B).
    Nrf54l,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct Nrf52SpiRegs {
    // EVENTS — HW-set only; SW may only write 0 to clear
    events_stopped: u32,
    events_endrx: u32,
    events_end: u32,
    events_endtx: u32,
    /// EVENTS_STARTED — transfer started. Unmodelled on the nRF52 map (no
    /// driver there polls it); decoded on the nRF54L map so a driver that waits
    /// on it makes progress instead of spinning.
    events_started: u32,
    /// Legacy SPI (NRF_SPI_Type) EVENTS_READY @ 0x108 — TXD byte done / RXD ready.
    events_ready: u32,
    /// Legacy SPI RXD @ 0x518 (last byte clocked in).
    legacy_rxd: u32,

    // INTEN — bit-field enabling each event's IRQ
    inten: u32,
    /// SHORTS (nRF54L 0x200) — event-to-task shortcuts. Stored for readback;
    /// no shortcut is acted on, because the modelled transfer completes inside
    /// one call and there is no window in which a shortcut could re-arm it.
    shorts: u32,

    // Config / pin-select / mode
    enable: u32,
    psel_sck: u32,
    psel_mosi: u32,
    psel_miso: u32,
    /// PSEL.CSN (0x514). Corroborated on live nRF52840 silicon 2026-08-09:
    /// writing 0x2B to 0x40003514 reads 0x2B back, so the register is real and
    /// storing. It was absent from this layout, so the serial-instance window
    /// returned 0 for it.
    psel_csn: u32,
    frequency: u32,
    config: u32,

    // EasyDMA descriptors
    rxd_ptr: u32,
    rxd_maxcnt: u32,
    rxd_amount: u32,
    txd_ptr: u32,
    txd_maxcnt: u32,
    txd_amount: u32,

    // Over-read character (low 8 bits, rest reserved)
    orc: u32,

    /// Which generation's offsets this instance decodes. Default `Nrf52` so
    /// every existing construction site keeps its behaviour unchanged.
    map: NrfSpimMap,

    // ── nRF54L-only registers ──────────────────────────────────────────────
    /// PRESCALER.DIVISOR (0x52C, reset 0x40). A real core-clock divisor, not
    /// the nRF52 FREQUENCY enumeration.
    prescaler: u32,
    /// PSEL.DCX (nRF54L 0x60C, nRF52 `PSELDCX` 0x56C) — the data/command
    /// line the SPIM drives ITSELF. A disconnected select (bit 31 set) leaves
    /// D/C to a plain GPIO, which is how the nRF52-era boards wire a display.
    psel_dcx: u32,
    /// DCXCNT (nRF54L 0x5B4, nRF52 0x570) — "the number of command bytes
    /// preceding the data bytes. The PSEL.DCX line will be low during
    /// transmission of command bytes" (SVD GLOBAL_SPIM00.DCXCNT). 0 means the
    /// whole transfer is data; on the nRF52 SPIM3 0xF means all of it is
    /// command.
    dcxcnt: u32,
    /// CSNPOL (nRF54L 0x5B8, nRF52 0x568) — polarity of the hardware CSN
    /// output: 0 active low, 1 active high.
    csnpol: u32,
    /// IFTIMING.CSNDUR (nRF54L 0x5B0, nRF52 0x564, reset 2) — CSN to SCK and
    /// CSN-high time in 64 MHz cycles. Timed on a world `gpio_net` by the
    /// nRF52 SPIM3 net engine (`spi_nrf_net`), stored for readback otherwise.
    csndur: u32,
    /// IFTIMING.RXDELAY (nRF54L 0x5AC, nRF52 0x560, reset 2) — stored for
    /// readback.
    rxdelay: u32,
}

/// `ENABLE` value that selects the SPIM personality on the shared
/// SPIM/SPIS/SPI/TWIM/TWI/TWIS window (nRF52840 PS v1.11 §6.25.6.17, p733).
const NRF52_ENABLE_SPIM: u32 = 7;
/// Nordic legacy SPI (NRF_SPI_Type) ENABLE value — Arduino SPI library uses this,
/// not SPIM EasyDMA. PS: SPI_ENABLE_ENABLE_Enabled = 1.
const NRF52_ENABLE_SPI: u32 = 1;
/// `CONFIG` fields (nRF52840 PS v1.11 §6.25.6.22, p737).
const NRF52_CONFIG_ORDER_LSB: u32 = 1 << 0;
const NRF52_CONFIG_CPHA: u32 = 1 << 1;
const NRF52_CONFIG_CPOL: u32 = 1 << 2;
/// Buffered MOSI bytes past which a transfer's narration is dropped rather than
/// truncated. See [`Spi::nrf52_wire_flush`].
const NRF52_WIRE_BYTE_CAP: usize = 2_048;

/// Held MOSI bytes past which the EFR32 Series-2 USART's wire narration is
/// dropped rather than held any longer. See [`Spi::efr32_wire_flush`].
///
/// ⚠️ THE ONLY THING BOUNDING THIS BUFFER. Every other wire buffer in this file
/// already has a ceiling — [`NRF52_WIRE_BYTE_CAP`] above and
/// [`H5_WIRE_BURST_CAP`] — and the EFR32 path had none. A run is held whenever
/// `emit_between` cannot fit it in the `cursor..now` window; with no
/// logic-capture tap installed `PadLines::tap_clock()` is `None`, `now` reads 0
/// forever, and the window is therefore never open. Without this cap the buffer
/// grew by one entry per transmitted byte and every push re-narrated all of it,
/// so shifting n bytes cost O(n^2): a 108 800-byte ST7789 screen fill on
/// BRD2709A dropped the engine from 4 000 000 to 1 400 cycles/s and the
/// playground's first frame never returned.
///
/// Sized like [`H5_WIRE_BURST_CAP`], and for the same reason: a run longer than
/// this has more edges than any analyzer window shows, so holding it buys
/// nothing a reader can see.
const EFR32_WIRE_BYTE_CAP: usize = 256;

/// INTEN bit positions (PS §6.30 INTEN register).
/// STOPPED=1, ENDRX=4, END=6, ENDTX=8.
const INTEN_STOPPED: u32 = 1 << 1;
const INTEN_ENDRX: u32 = 1 << 4;
const INTEN_END: u32 = 1 << 6;
const INTEN_ENDTX: u32 = 1 << 8;

/// nRF54L INTENSET bit positions (SVD GLOBAL_SPIM00.INTENSET).
///
/// ⚠️ Only STOPPED coincides with the nRF52 numbering. END moved 6 → 2, and the
/// two EasyDMA completions moved out to bits 19 and 26. Reusing the nRF52
/// constants here would enable the wrong lines: an `INTENSET = 1 << 2` from a
/// driver asking for END would read as the nRF52 map's *nothing*, and the
/// transfer-complete interrupt would never be delivered.
const NRF54L_INTEN_STARTED: u32 = 1 << 0;
const NRF54L_INTEN_STOPPED: u32 = 1 << 1;
const NRF54L_INTEN_END: u32 = 1 << 2;
const NRF54L_INTEN_DMA_RX_END: u32 = 1 << 19;
const NRF54L_INTEN_DMA_TX_END: u32 = 1 << 26;

impl Nrf52SpiRegs {
    /// A reset-state register file on the nRF52 offset map.
    ///
    /// The SPIM3-era registers take their SVD reset values
    /// (`configs/peripherals/nrf52840/spim3.yaml`): `PSEL.CSN` and
    /// `PSELDCX` DISCONNECTED, `IFTIMING.RXDELAY` and `IFTIMING.CSNDUR` 2. A
    /// zero `PSEL.CSN` would name P0.00, which the SPIM3 net engine would
    /// then drive as a chip select nobody asked for.
    fn new_nrf52() -> Self {
        Self {
            psel_csn: 0xFFFF_FFFF,
            psel_dcx: 0xFFFF_FFFF,
            rxdelay: 2,
            csndur: 2,
            ..Self::default()
        }
    }

    /// A reset-state register file on the nRF54L offset map.
    ///
    /// Not `Default`: PRESCALER resets to 0x40 (SVD GLOBAL_SPIM00.PRESCALER)
    /// and every PSEL resets DISCONNECTED (bit 31). Zero-filling those would
    /// claim pad 0 for SCK and divide the 128 MHz source by nothing.
    fn new_nrf54l() -> Self {
        Self {
            map: NrfSpimMap::Nrf54l,
            prescaler: 0x0000_0040,
            psel_sck: 0xFFFF_FFFF,
            psel_mosi: 0xFFFF_FFFF,
            psel_miso: 0xFFFF_FFFF,
            psel_csn: 0xFFFF_FFFF,
            psel_dcx: 0xFFFF_FFFF,
            ..Self::default()
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match self.map {
            NrfSpimMap::Nrf52 => self.read_reg_nrf52(offset),
            NrfSpimMap::Nrf54l => self.read_reg_nrf54l(offset),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) -> bool {
        match self.map {
            NrfSpimMap::Nrf52 => self.write_reg_nrf52(offset, value),
            NrfSpimMap::Nrf54l => self.write_reg_nrf54l(offset, value),
        }
    }

    fn read_reg_nrf52(&self, offset: u64) -> u32 {
        match offset {
            // TASKS read as 0 (write-only strobes on silicon)
            0x010 | 0x014 => 0,
            // EVENTS
            0x104 => self.events_stopped,
            0x108 => self.events_ready, // legacy SPI EVENTS_READY
            0x110 => self.events_endrx,
            0x118 => self.events_end,
            0x120 => self.events_endtx,
            // EVENTS_STARTED: set by the net engine (`spi_nrf_net`) only.
            0x14C => self.events_started,
            // INTEN / INTENSET / INTENCLR all mirror the inten value
            0x300 | 0x304 | 0x308 => self.inten,
            // Config
            0x500 => self.enable,
            0x508 => self.psel_sck,
            0x50C => self.psel_mosi,
            0x510 => self.psel_miso,
            0x514 => self.psel_csn,
            0x518 => self.legacy_rxd, // legacy SPI RXD
            0x51C => 0,               // legacy SPI TXD (write-only on silicon)
            0x524 => self.frequency,
            0x554 => self.config,
            // SPIM3's hardware chip select and D/CX (PS v1.x §6.25.7; the
            // SVD lists them on every SPIM instance).
            0x560 => self.rxdelay & 0x7,
            0x564 => self.csndur & 0xFF,
            0x568 => self.csnpol & 0x1,
            0x56C => self.psel_dcx,
            0x570 => self.dcxcnt & 0xF,
            // EasyDMA descriptors
            0x534 => self.rxd_ptr,
            0x538 => self.rxd_maxcnt,
            0x53C => self.rxd_amount,
            0x544 => self.txd_ptr,
            0x548 => self.txd_maxcnt,
            0x54C => self.txd_amount,
            // ORC
            0x5C0 => self.orc & 0xFF,
            _ => {
                crate::census_reg!("spi:Nrf52SpiRegs", offset, "read");
                0
            }
        }
    }

    /// Handle MMIO writes for the nRF52 SPIM register file.
    ///
    /// Returns `true` when TASKS_START was triggered (so the caller can set
    /// `pending_start`). TASKS_STOP returns `false` (handled here).
    ///
    /// EVENTS write semantics: SW write of 1 is a no-op (only HW sets events);
    /// SW write of 0 clears the event.
    fn write_reg_nrf52(&mut self, offset: u64, value: u32) -> bool {
        match offset {
            // TASKS — trigger on non-zero write
            0x010 => return value != 0, // TASKS_START: signal caller
            0x014 => {
                // TASKS_STOP: no state needed; events_stopped set by HW
            }

            // EVENTS — SW write of 1 ignored; SW write of 0 clears
            0x104 if value == 0 => self.events_stopped = 0,
            0x108 if value == 0 => self.events_ready = 0,
            0x110 if value == 0 => self.events_endrx = 0,
            0x118 if value == 0 => self.events_end = 0,
            0x120 if value == 0 => self.events_endtx = 0,
            0x14C if value == 0 => self.events_started = 0,

            // INTEN (direct write)
            0x300 => self.inten = value,
            // INTENSET (set bits)
            0x304 => self.inten |= value,
            // INTENCLR (clear bits)
            0x308 => self.inten &= !value,

            // Config / pin-select
            0x500 => self.enable = value,
            0x508 => self.psel_sck = value,
            0x50C => self.psel_mosi = value,
            0x510 => self.psel_miso = value,
            0x514 => self.psel_csn = value,
            0x524 => self.frequency = value,
            0x554 => self.config = value,
            0x560 => self.rxdelay = value & 0x7,
            0x564 => self.csndur = value & 0xFF,
            0x568 => self.csnpol = value & 0x1,
            0x56C => self.psel_dcx = value,
            0x570 => self.dcxcnt = value & 0xF,

            // EasyDMA descriptors (AMOUNT registers are HW-written; firmware
            // should not write them, but the model accepts writes so firmware
            // that does an initialising clear doesn't get confused)
            0x534 => self.rxd_ptr = value,
            0x538 => self.rxd_maxcnt = value,
            0x53C => self.rxd_amount = value,
            0x544 => self.txd_ptr = value,
            0x548 => self.txd_maxcnt = value,
            0x54C => self.txd_amount = value,

            // ORC (only low 8 bits are meaningful)
            0x5C0 => self.orc = value & 0xFF,

            _ => {
                crate::census_reg!("spi:Nrf52SpiRegs", offset, "write");
            }
        }
        false
    }

    /// nRF54L-family SPIM register file.
    ///
    /// Offsets transcribed from the Nordic MDK `nrf54lm20a_application.svd`,
    /// peripheral `GLOBAL_SPIM00` (the map is family-wide, not part-specific —
    /// SPIM20/21/22/23/24/30 derive from the same description).
    ///
    /// ⚠️ 0x010 is TASKS_RESUME here and TASKS_START on the nRF52 map. Decoding
    /// an nRF54L instance with the nRF52 table therefore turns every driver's
    /// "start" into a resume of a transfer that was never suspended, which is
    /// silent: the enable and the mode write both land, and nothing moves.
    fn read_reg_nrf54l(&self, offset: u64) -> u32 {
        match offset {
            // TASKS read as 0 (write-only strobes on silicon)
            0x000 | 0x004 | 0x00C | 0x010 => 0,
            // EVENTS
            0x100 => self.events_started,
            0x104 => self.events_stopped,
            0x108 => self.events_end,
            0x14C => self.events_endrx, // EVENTS_DMA.RX.END
            0x168 => self.events_endtx, // EVENTS_DMA.TX.END
            // SHORTS
            0x200 => self.shorts,
            // INTENSET / INTENCLR both read the enable mask. There is no
            // INTEN at 0x300 on this family — the SVD lists only these two.
            0x304 | 0x308 => self.inten,
            // Config
            0x500 => self.enable,
            0x52C => self.prescaler,
            0x554 => self.config,
            0x5AC => self.rxdelay,
            0x5B0 => self.csndur,
            0x5B4 => self.dcxcnt,
            0x5B8 => self.csnpol,
            0x5C0 => self.orc & 0xFF,
            // Pin select
            0x600 => self.psel_sck,
            0x604 => self.psel_mosi,
            0x608 => self.psel_miso,
            0x60C => self.psel_dcx,
            0x610 => self.psel_csn,
            // EasyDMA descriptors
            0x704 => self.rxd_ptr,
            0x708 => self.rxd_maxcnt,
            0x70C => self.rxd_amount,
            0x73C => self.txd_ptr,
            0x740 => self.txd_maxcnt,
            0x744 => self.txd_amount,
            _ => {
                crate::census_reg!("spi:Nrf54lSpimRegs", offset, "read");
                0
            }
        }
    }

    /// Handle MMIO writes on the nRF54L offset map.
    ///
    /// Returns `true` when TASKS_START (0x000) was triggered.
    fn write_reg_nrf54l(&mut self, offset: u64, value: u32) -> bool {
        match offset {
            0x000 => return value != 0, // TASKS_START
            0x004 => {}                 // TASKS_STOP — events_stopped is HW-set
            0x00C | 0x010 => {}         // TASKS_SUSPEND / TASKS_RESUME

            // EVENTS — SW write of 1 ignored; SW write of 0 clears
            0x100 if value == 0 => self.events_started = 0,
            0x104 if value == 0 => self.events_stopped = 0,
            0x108 if value == 0 => self.events_end = 0,
            0x14C if value == 0 => self.events_endrx = 0,
            0x168 if value == 0 => self.events_endtx = 0,

            0x200 => self.shorts = value,
            0x304 => self.inten |= value,  // INTENSET
            0x308 => self.inten &= !value, // INTENCLR

            0x500 => self.enable = value,
            0x52C => self.prescaler = value,
            0x554 => self.config = value,
            0x5AC => self.rxdelay = value,
            0x5B0 => self.csndur = value,
            0x5B4 => self.dcxcnt = value,
            0x5B8 => self.csnpol = value,
            0x5C0 => self.orc = value & 0xFF,

            0x600 => self.psel_sck = value,
            0x604 => self.psel_mosi = value,
            0x608 => self.psel_miso = value,
            0x60C => self.psel_dcx = value,
            0x610 => self.psel_csn = value,

            0x704 => self.rxd_ptr = value,
            0x708 => self.rxd_maxcnt = value,
            0x70C => self.rxd_amount = value,
            0x73C => self.txd_ptr = value,
            0x740 => self.txd_maxcnt = value,
            0x744 => self.txd_amount = value,

            _ => {
                crate::census_reg!("spi:Nrf54lSpimRegs", offset, "write");
            }
        }
        false
    }
}

// ── NXP Kinetis DSPI (KW41Z SPI0/SPI1) ──────────────────────────────────────
// MCR@0x0, TCR@0x8, CTAR0@0xC, CTAR1@0x10, SR@0x2C, RSER@0x30, PUSHR@0x34,
// POPR@0x38. A frame is sent by writing PUSHR; the `fsl_dspi` blocking write
// (DSPI_MasterWriteDataBlocking) clears SR.TCF, spins until SR.TFFF (TX FIFO
// has room — always true here), writes PUSHR, then spins until SR.TCF. We model
// a depth-immaterial FIFO: TFFF stays asserted, and each PUSHR write completes
// the frame synchronously (broadcast to attached devices) and raises TCF.
const DSPI_SR_RFDF: u32 = 0x0002_0000;
const DSPI_SR_TFFF: u32 = 0x0200_0000;
const DSPI_SR_EOQF: u32 = 0x1000_0000;
const DSPI_SR_TCF: u32 = 0x8000_0000;

#[derive(Debug, Clone, serde::Serialize)]
struct KinetisDspiRegs {
    mcr: u32,
    tcr: u32,
    ctar: [u32; 2],
    sr: u32,
    rser: u32,
    /// Last byte clocked back on MISO (POP RX FIFO). 0 for a write-only device.
    popr: u32,
}

impl Default for KinetisDspiRegs {
    fn default() -> Self {
        Self {
            // HALT=1 at reset (module stopped until firmware configures + clears
            // it); TFFF asserted so the first DSPI_GetStatusFlags poll passes.
            mcr: 0x0000_0001,
            tcr: 0,
            ctar: [0, 0],
            sr: DSPI_SR_TFFF,
            rser: 0,
            popr: 0,
        }
    }
}

impl KinetisDspiRegs {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.mcr,
            0x08 => self.tcr,
            0x0C => self.ctar[0],
            0x10 => self.ctar[1],
            0x2C => self.sr,
            0x30 => self.rser,
            0x38 => self.popr,
            _ => {
                crate::census_reg!("spi:KinetisDspiRegs", offset, "read");
                0
            }
        }
    }
}

/// Which datasheet alternate-function map routes this controller's pads.
///
/// # Why the register layout cannot answer this
///
/// For the classic/FIFO STM32 SPI the register layout DOES pick the table
/// (`is_fifo_layout` selects L4 over F4), because those two families' AF maps
/// agree wherever they overlap. The H5 "SPI v3" IP breaks that: the STM32H563,
/// STM32H735 and STM32WBA52 all carry the identical `profile: "stm32h5"`
/// register file and DISAGREE about what their pads mean.
///
/// Concretely, and this is the whole reason this enum exists:
///
/// | pad | H563 / H735 (AF5) | WBA52 (AF5) |
/// |-----|-------------------|-------------|
/// | PB3 | `SPI1_SCK`        | `SPI1_MISO` |
/// | PB4 | `SPI1_MISO`       | `SPI1_SCK`  |
///
/// Same port, same pin, same AF nibble, SCK and MISO exactly swapped. Picking
/// the wrong table does not fail — it publishes the clock onto the pad carrying
/// data and vice versa, and a decoder reads confident garbage. That is the
/// silent wrong-pad failure the F4/L4 I²C table split exists to prevent, and
/// the "per-family AF map keyed on something finer than the register layout"
/// that `wire_stm32_uart_pads` documents as a known gap.
///
/// So the map is DECLARED in the chip yaml (`config: { pad_map: stm32h5 }`),
/// a per-part delta in the same spirit as `cr2_mask`/`cr1_mask`, and the
/// default is [`SpiPadMap::None`] — fail CLOSED. A new H5-profile part that
/// forgets the key gets no SPI pad routing (an honest gap, visible on the
/// bus-visibility board) rather than the H563's pinout silently applied to
/// silicon that does not have it. The STM32U575 is the case that keeps this
/// honest a third time: same register file again, third pinout
/// ([`SpiPadMap::Stm32U5`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub enum SpiPadMap {
    /// No declared AF map: publish no pads. The default, and the honest answer
    /// for any part whose pinout has not been read off its datasheet.
    #[default]
    None,
    /// STM32H5/H7 map — STM32H563 (DS14258 Rev 6 Table 15, pages 106-107) and
    /// STM32H735 (DS13312 Rev 4 Table 9, pages 96-99), which agree row for row
    /// on ports A and B.
    Stm32H5,
    /// STM32WBA map — STM32WBA52 (DS14127 Rev 10 Table 25, pages 76-77).
    Stm32Wba,
    /// STM32U5 map — STM32U575 (DS13737 Rev 5, Table 27 "Alternate function
    /// AF0 to AF7", pages 125-133; cross-checked against the CubeMX-generated
    /// `PeripheralPins_NUCLEO_U575ZI_Q.c` in `framework-arduinoststm32`).
    ///
    /// A U5 part carries the same `profile: "stm32h5"` register file as the
    /// H563/H735, but its AF map is neither of theirs: SPI1's NUCLEO-U575ZI-Q
    /// pads are PA5 SCK / PA6 MISO / PA7 MOSI / PA4 NSS, all AF5, and SPI3
    /// puts MOSI on PD6 at AF5 while every other SPI3 row is AF6.
    Stm32U5,
    /// STM32G0 map — STM32G071 (DS12232, port A/B alternate-function tables,
    /// AF0 column): SPI1 on PA4 NSS / PA5 SCK / PA6 MISO / PA7 MOSI, PA15
    /// NSS, PB3 SCK / PB4 MISO / PB5 MOSI; SPI2 on PB12 NSS / PB13 SCK /
    /// PB14 MISO / PB15 MOSI, NSS included: on a world `gpio_net` the
    /// hardware slave select is a real wire.
    Stm32G0,
    /// STM32G4 map — STM32G47x (ST open pin data
    /// `GPIO-STM32G47x_gpio_v1_0_Modes.xml`): SPI1/SPI2 on AF5, SPI3 on AF6,
    /// NSS included. Not the L4 table: PE12-PE15/AF5 are SPI4 on a G4.
    Stm32G4,
    /// STM32WB map — STM32WB55 (ST open pin data
    /// `GPIO-STM32WB55x_gpio_v1_0_Modes.xml`): SPI1/SPI2 on AF5 (PC1 and PD3
    /// at AF3), NSS included.
    Stm32Wb,
    /// STM32L0 map — STM32L07x (ST open pin data
    /// `GPIO-STM32L071_gpio_v1_0_Modes.xml`): SPI1 on AF0 (PE12-PE15 on
    /// AF2), SPI2 on AF0/AF1/AF2/AF5 by pad, NSS included. A classic-layout
    /// part, which without this falls through to the F4 AF5 table.
    Stm32L0,
    /// The F4 AF5 table, declared explicitly. A classic-layout part gets it
    /// by default; a FIFO-layout part would otherwise get the L4 table, so an
    /// F7 (FIFO SPI, F4 pinout) names this one.
    Stm32F4,
}

impl FromStr for SpiPadMap {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "stm32h5" | "h5" | "stm32h7" | "h7" => Ok(Self::Stm32H5),
            "stm32wba" | "wba" => Ok(Self::Stm32Wba),
            "stm32u5" | "u5" => Ok(Self::Stm32U5),
            "stm32g0" | "g0" => Ok(Self::Stm32G0),
            "stm32f4" | "f4" => Ok(Self::Stm32F4),
            "stm32g4" | "g4" => Ok(Self::Stm32G4),
            "stm32wb" | "wb" => Ok(Self::Stm32Wb),
            "stm32l0" | "l0" => Ok(Self::Stm32L0),
            "none" => Ok(Self::None),
            other => Err(anyhow::anyhow!(
                "unsupported SPI pad_map '{other}'; supported: stm32h5, stm32wba, stm32u5, stm32g0, stm32f4, stm32g4, stm32wb, stm32l0, none"
            )),
        }
    }
}

/// Silicon Labs EFR32 Series-2 USART registers, synchronous (SPI) mode.
///
/// # Sources
///
/// Offsets walked from `USART_TypeDef` in `efr32mg26_usart.h` (`simplicity_sdk`
/// tag `sisdk-2025.6`); bit positions and `STATUS`'s `0x2040` reset value are
/// the `_USART_…_SHIFT` / `_RESETVALUE` defines from the same header.
///
/// ⚠️ This is the SAME register block the console UART model drives — Series 2
/// has no separate SPI peripheral. `CTRL.SYNC` is what makes it SPI, and a
/// USART instance is one or the other, never both at once. The chip yaml must
/// therefore declare a given instance as `uart` or as `spi`, not both, and
/// USART1 on the BRD2709A is the VCOM console.
#[derive(Debug, Clone, serde::Serialize)]
struct Efr32s2SpiRegs {
    en: u32,
    ctrl: u32,
    frame: u32,
    trigctrl: u32,
    clkdiv: u32,
    iflag: u32,
    ien: u32,
    /// Last byte clocked back on MISO. 0 for a write-only device.
    rxdata: u32,
    /// `CMD` latches these: RX/TX enabled, and master mode.
    rx_enabled: bool,
    tx_enabled: bool,
    master: bool,
    /// A received byte is waiting in `RXDATA`.
    rxdatav: bool,
    /// `I2SCTRL` (0x54). Reset 0x0 per RM section 20.5.22 p.669 -- this block
    /// is a UART until told otherwise, and I2S is one more thing it must be
    /// TOLD to be, on top of CTRL.SYNC.
    i2sctrl: u32,
    /// True while the next RX word belongs to the RIGHT channel. RM section
    /// 20.3.3.11 p.632: "the USART always starts transmitting on the LEFT
    /// channel after being enabled", so this starts false and flips per word.
    i2s_right: bool,
    /// `STATUS.TXC` — a transmission has completed. **Clear out of reset**:
    /// silicon reads `STATUS = 0x2040` (TXBL | TXIDLE) with TXC low, because
    /// nothing has been transmitted yet. Measured on BRD2709A over SWD.
    txc: bool,
}

/// ⚠️ NOT `#[derive(Default)]`. Two of these fields have a non-zero silicon
/// reset value, and a derived `Default` silently disagreed with the die on
/// both — see the capture in `scripts/hw-oracle/captures/efr32mg26/`.
impl Default for Efr32s2SpiRegs {
    fn default() -> Self {
        Self {
            en: 0,
            ctrl: 0,
            frame: EFR_USART_FRAME_RESET,
            trigctrl: 0,
            clkdiv: 0,
            iflag: EFR_USART_IF_RESET,
            ien: 0,
            rxdata: 0,
            rx_enabled: false,
            tx_enabled: false,
            master: false,
            rxdatav: false,
            i2sctrl: 0,
            i2s_right: false,
            txc: false,
        }
    }
}

// Offsets, walked from `USART_TypeDef`.
const EFR_USART_IPVERSION: u64 = 0x00;
const EFR_USART_EN: u64 = 0x04;
const EFR_USART_CTRL: u64 = 0x08;
const EFR_USART_FRAME: u64 = 0x0C;
const EFR_USART_TRIGCTRL: u64 = 0x10;
const EFR_USART_CMD: u64 = 0x14;
const EFR_USART_STATUS: u64 = 0x18;
const EFR_USART_CLKDIV: u64 = 0x1C;
const EFR_USART_RXDATA: u64 = 0x24;
const EFR_USART_TXDATA: u64 = 0x3C;
const EFR_USART_IF: u64 = 0x48;
const EFR_USART_IEN: u64 = 0x4C;
/// I2S Control Register. RM (EFR32xG26, Rev 1.0) section 20.5.22 p.669;
/// reset 0x00000000, every field stated as 0x0 in that table.
const EFR_USART_I2SCTRL: u64 = 0x54;

const EFR_USART_EN_EN: u32 = 1 << 0;
/// I2SCTRL.EN -- "Enable I2S Mode". RM section 20.5.22 p.669.
const EFR_USART_I2SCTRL_EN: u32 = 1 << 0;
/// I2SCTRL.MONO -- "Switch between stereo and mono mode. Set for mono".
const EFR_USART_I2SCTRL_MONO: u32 = 1 << 1;
/// I2SCTRL.FORMAT, bits 10:8. Value 2 is W32D24 -- "32-bit word, 24-bit data",
/// which is exactly what an INMP441 puts on the wire.
const EFR_USART_I2SCTRL_FORMAT_SHIFT: u32 = 8;
const EFR_USART_I2SCTRL_FORMAT_MASK: u32 = 0b111 << EFR_USART_I2SCTRL_FORMAT_SHIFT;
const EFR_USART_CTRL_SYNC: u32 = 1 << 0;
/// `CTRL.CLKPOL` — the level SCK rests at between frames (SPI's CPOL).
const EFR_USART_CTRL_CLKPOL: u32 = 1 << 8;
/// `CTRL.CLKPHA` — which clock edge samples (SPI's CPHA).
const EFR_USART_CTRL_CLKPHA: u32 = 1 << 9;
/// `CTRL.MSBF` — MSB first. The narrator draws MSB-first only, so an LSB-first
/// frame publishes no waveform rather than a wrong one.
const EFR_USART_CTRL_MSBF: u32 = 1 << 10;
const EFR_USART_CMD_RXEN: u32 = 1 << 0;
const EFR_USART_CMD_RXDIS: u32 = 1 << 1;
const EFR_USART_CMD_TXEN: u32 = 1 << 2;
const EFR_USART_CMD_TXDIS: u32 = 1 << 3;
const EFR_USART_CMD_MASTEREN: u32 = 1 << 4;
const EFR_USART_CMD_MASTERDIS: u32 = 1 << 5;
const EFR_USART_CMD_CLEARRX: u32 = 1 << 11;

const EFR_USART_STATUS_RXENS: u32 = 1 << 0;
const EFR_USART_STATUS_TXENS: u32 = 1 << 1;
const EFR_USART_STATUS_MASTER: u32 = 1 << 2;
const EFR_USART_STATUS_TXC: u32 = 1 << 5;
const EFR_USART_STATUS_TXBL: u32 = 1 << 6;
const EFR_USART_STATUS_RXDATAV: u32 = 1 << 7;

/// `_USART_STATUS_RESETVALUE` = TXBL | TXIDLE. TXC is **not** in it.
const EFR_USART_STATUS_RESET: u32 = 0x0000_2040;
/// `_USART_IPVERSION_RESETVALUE`. ⚠️ This was modelled as 2 — a guess, not the
/// header. `efr32mg26_usart.h` says 0 and the die reads 0.
const EFR_USART_IPVERSION_RESET: u32 = 0;
/// `_USART_FRAME_RESETVALUE` = 8 data bits, 1 stop bit, no parity.
const EFR_USART_FRAME_RESET: u32 = 0x0000_1005;
/// `_USART_IF_RESETVALUE` = TXBL. The transmit buffer is empty out of reset, so
/// its "level" flag is already up; the die confirms `IF = 0x2`.
const EFR_USART_IF_RESET: u32 = EFR_USART_IF_TXBL;

/// `IF.TXC` / `IF.TXBL` / `IF.RXDATAV`, the three a polled SPI driver watches.
const EFR_USART_IF_TXC: u32 = 1 << 0;
const EFR_USART_IF_TXBL: u32 = 1 << 1;
const EFR_USART_IF_RXDATAV: u32 = 1 << 2;

impl Efr32s2SpiRegs {
    fn status(&self) -> u32 {
        // TXBL rides the reset word: a frame completes inside the TXDATA write
        // here, so the shift register is never occupied. TXC does NOT — it
        // means "a transmission finished", and out of reset none has.
        let mut s = EFR_USART_STATUS_RESET;
        if self.rx_enabled {
            s |= EFR_USART_STATUS_RXENS;
        }
        if self.tx_enabled {
            s |= EFR_USART_STATUS_TXENS;
        }
        if self.master {
            s |= EFR_USART_STATUS_MASTER;
        }
        if self.rxdatav {
            s |= EFR_USART_STATUS_RXDATAV;
        }
        if self.txc {
            s |= EFR_USART_STATUS_TXC;
        }
        s | EFR_USART_STATUS_TXBL
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            EFR_USART_IPVERSION => EFR_USART_IPVERSION_RESET,
            EFR_USART_EN => self.en,
            EFR_USART_CTRL => self.ctrl,
            EFR_USART_FRAME => self.frame,
            EFR_USART_TRIGCTRL => self.trigctrl,
            EFR_USART_CMD => 0, // write-only
            EFR_USART_STATUS => self.status(),
            EFR_USART_CLKDIV => self.clkdiv,
            EFR_USART_RXDATA => self.rxdata,
            EFR_USART_IF => self.iflag,
            EFR_USART_IEN => self.ien,
            EFR_USART_I2SCTRL => self.i2sctrl,
            _ => {
                crate::census_reg!("spi:Efr32s2SpiRegs", offset, "read");
                0
            }
        }
    }
}

/// Family-isolated SPI register state. STM32 and nRF register sets cannot
/// coexist on one instance.
#[derive(Debug, Clone, serde::Serialize)]
enum SpiRegs {
    Stm32(Stm32SpiRegs),
    Stm32H5(Stm32H5SpiRegs),
    Nrf52(Nrf52SpiRegs),
    KinetisDspi(KinetisDspiRegs),
    Efr32s2Usart(Efr32s2SpiRegs),
}

impl Default for SpiRegs {
    fn default() -> Self {
        SpiRegs::Stm32(Stm32SpiRegs::default())
    }
}

/// SPI peripheral: family-isolated registers (`regs`) + a shared transfer
/// engine and attached-device routing.
#[derive(Default, serde::Serialize)]
pub struct Spi {
    regs: SpiRegs,

    /// True for the FIFO-equipped STM32 SPI (L4/F7/G4). On those parts RXNE
    /// asserts only when the RX FIFO reaches the CR2.FRXTH threshold, so a
    /// single 8-bit frame with FRXTH=0 (the reset default, threshold = 16 bit)
    /// leaves RXNE CLEAR. Verified against a real NUCLEO-L476RG: SR reads
    /// 0x0002 after a transmit with no slave wired. The classic F1/F4 port has
    /// no FIFO and sets RXNE on every completed frame.
    rx_fifo: bool,
    /// The modelled 32-bit RX FIFO (FIFO layout only). In a `Cell` because a
    /// DR read (`&self`) pops it.
    fifo_rx: Cell<RxFifo>,
    /// FIFO layout SR.OVR, and whether DR has been read since it was set:
    /// OVR clears on "a read of SPI_DR followed by a read of SPI_SR" (RM0444
    /// §35.5.10), both `&self` accesses, hence the cells.
    fifo_ovr: Cell<bool>,
    fifo_ovr_dr_read: Cell<bool>,

    /// MOSI bytes this EFR32 USART has shifted but not yet narrated, and the
    /// cycle an earlier flush of this same wire already ran to.
    ///
    /// ⚠️ BUFFERED, not per byte. Narrating each `TXDATA` write on its own
    /// looked right — firmware writes a Series-2 USART a byte at a time, so
    /// each byte really does have its own moment — and it is wrong the moment
    /// firmware writes two bytes without stepping the machine between them.
    /// Every frame then lands on the same cycle and they stack: the
    /// `bus_visibility` ratchet decoded three bytes as ONE (`[c1]` from
    /// `[53, 1c, e1]`), which is a waveform describing a transfer that never
    /// happened. Buffering and emitting the run between the cursor and now is
    /// what the H5 path already does, for the same reason.
    #[serde(skip)]
    efr32_wire_bytes: Vec<u8>,
    #[serde(skip)]
    efr32_wave_cursor: u64,
    /// A publication wakeup is already armed for the held burst.
    #[serde(skip)]
    efr32_scheduled: bool,

    /// Classic/FIFO STM32 RXNE is **clear-on-DR-read** (RM0008 / RM0351).
    /// `Peripheral::read` is `&self`, so the flag lives in a `Cell` and SR
    /// reads merge it into bit 0. Without this, RXNE stayed set after the first
    /// frame and Arduino `SPI.transfer()` re-read a stale DR (matrix L4 saw
    /// `0x00019016` = default MAX31855 frame shifted by one residual byte).
    #[serde(skip)]
    stm32_rxne: Cell<bool>,

    // STM32 bit-engine state (classic/FIFO layout only; the other register
    // families keep their own transfer semantics).
    /// The frame currently clocking on the wire, if any.
    frame: Option<ActiveFrame>,
    /// Frames queued behind the wire (FIFO data packing, back-to-back DR
    /// writes). Each entry is one full frame value.
    tx_queue: std::collections::VecDeque<u16>,
    #[serde(skip)]
    scheduled: bool,
    /// Event-scheduler path only: the absolute CPU cycle the engine's wire
    /// state corresponds to. Anchored by `sync_to` (called by the bus with
    /// `current_cycle` before every MMIO write, so a DR write pins the frame
    /// start to the batch-start cycle — identically in clamped and batched
    /// runs) and advanced by `on_event`. The legacy walk clocks the engine
    /// through `tick_elapsed` instead and never touches this.
    #[serde(skip)]
    anchor_tick: u64,
    /// Shared SCK/MOSI/MISO line levels, read by the STM32 GPIO model for
    /// AF-routed pads. Created lazily by [`Self::line_levels_arc`] at bus
    /// wiring time; `None` when no pads are wired (the engine still runs —
    /// only the wire publication is skipped).
    #[serde(skip)]
    lines: Option<Arc<SpiLineLevels>>,
    /// When true, completed transfers also load the transmitted frame into the
    /// RX path (`dr` + RXNE), as if MOSI were jumpered to MISO. Defaults false.
    loopback: bool,

    /// nRF52 SPIM: set when TASKS_START is written; cleared by the EasyDMA
    /// engine via either `tick_with_bus` (bare-bus / bus_tick_indices) or
    /// `on_event` (Machine + event-scheduler, delay-0).
    #[serde(skip)]
    nrf52_pending_start: bool,

    /// nRF52 SPIM ONLY: this instance's standing claims on the pads
    /// `PSEL.SCK` / `PSEL.MOSI` name. Nordic muxes at the peripheral, so THIS
    /// is what decides which pad reads the wire published into `lines` — there
    /// is no AF nibble at the port to ask. See
    /// [`crate::peripherals::nrf52::pin_select`].
    #[serde(skip)]
    nrf52_claim_sck: crate::peripherals::nrf52::pin_select::NrfPinClaim,
    #[serde(skip)]
    nrf52_claim_mosi: crate::peripherals::nrf52::pin_select::NrfPinClaim,
    /// `PSEL.MISO`'s claim: an input to the SPIM, routed so that a pad on a
    /// world `gpio_net` hands the net's level to the bit engine
    /// (`spi_nrf_net`).
    #[serde(skip)]
    nrf52_claim_miso: crate::peripherals::nrf52::pin_select::NrfPinClaim,
    /// nRF52 SPIM3 only: `PSEL.CSN`'s and `PSELDCX`'s claims, the hardware
    /// chip select and data/command lines this SPIM drives itself on a world
    /// `gpio_net` (`spi_nrf_net`). Installed by the wiring pass for the SPIM3
    /// instance alone ([`Self::nrf52_hw_select`]); SPIM0-2 have no hardware
    /// select, so their `PSEL.CSN` stays a stored register.
    #[serde(skip)]
    nrf52_claim_csn: crate::peripherals::nrf52::pin_select::NrfPinClaim,
    #[serde(skip)]
    nrf52_claim_dcx: crate::peripherals::nrf52::pin_select::NrfPinClaim,
    /// This SPIM drives `CSN` and `DCX` (SPIM3, nRF52840 PS v1.x §6.25).
    #[serde(skip)]
    nrf52_hw_select: bool,
    /// The nRF52 SPIM's bit engine on a world `gpio_net` (`spi_nrf_net`).
    #[serde(skip)]
    nrf_net: nrf_net::SpimNet,

    /// Classic-SPI CR2 writable mask — a per-part delta on the shared classic
    /// layout. F1 implements 0xE7; F4 adds bit 4 (FRF, TI-mode) → 0xF7,
    /// silicon-confirmed on the bench F103 (0xE7) and F407 (0xF7). Set from the
    /// chip config's `cr2_mask`. Ignored by the FIFO layout (its own CR2 logic).
    cr2_mask: u32,

    /// Classic-SPI CR1 writable mask — a per-part delta. `None` = fully writable
    /// (0xFFFF), the default that matches F103/L0/L476 silicon (CR1 reads back
    /// 0xFFFF). F407 silicon does NOT latch CR1 bit 12 (CRCNEXT): writing
    /// 0xFFFF reads back 0xEFFF, so its chip config sets `cr1_mask: 0xEFFF`.
    cr1_mask: Option<u16>,

    /// CPOL/CPHA of the first attached device that opted into edge-accurate
    /// sampling ([`SpiSampling::Edge`]), resolved ONCE at attach time in
    /// [`Self::push_device`]. `None` — no device opted in — is the default and
    /// keeps the byte-level frame path byte for byte as it was; the engine
    /// tests this `Option` once per frame and never per bit.
    ///
    /// One mode per controller: the bus dispatcher broadcasts a frame to every
    /// attached device (last non-zero MISO wins), so the wire cannot carry two
    /// different slave phases at once. The first opt-in governs.
    #[serde(skip)]
    edge_slave: Option<(bool, bool)>,
    /// Edge path only: the (MOSI, MISO) levels the pads still carried when the
    /// last frame ended — what a slave latches if its sample edge lands on the
    /// very first boundary of the next frame. Held by the engine rather than
    /// read back from [`Self::lines`] so the edge model gives the same answer
    /// whether or not this controller's pads happen to be AF-routed.
    #[serde(skip)]
    edge_hold: (bool, bool),
    /// Declared alternate-function pad map (chip yaml
    /// `config: { pad_map: ... }`). Read by
    /// [`crate::bus::SystemBus::wire_stm32_spi_pads`] to pick the H5 AF table;
    /// see [`SpiPadMap`] for why the register layout cannot decide this.
    pad_map: SpiPadMap,

    // ── H5 "SPI v3" wire narration (Stm32H5 layout only) ────────────────────
    //
    // The H5 IP has NO bit engine: `write_stm32h5_reg` offset 0x20 moves a
    // whole frame inside the TXDR write (`ctsize -= 1`, EOT at zero), so there
    // is no `ticks_left` countdown and no bit index for the pads to follow.
    // The waveform is therefore NARRATED from the completed transfer, exactly
    // as the RP2040 PL022 does — see `peripherals::spi_waveform` for why that
    // is a faithful trace and what it cannot show.
    /// Frames pushed since the last narration flush, each with the CFG1/CFG2
    /// framing held AT THE MOMENT it was written, so firmware that reprograms
    /// DSIZE/CPOL/CPHA mid-burst still narrates each frame the way it went out.
    #[serde(skip)]
    h5_wire_words: Vec<(u16, SpiFraming)>,
    /// SCK period the buffered burst is narrated at, captured on its first
    /// frame. A rate change mid-burst force-flushes what is held rather than
    /// repainting it at a rate no transfer used.
    #[serde(skip)]
    h5_wire_bit_time: u64,
    /// Cycle the last H5 narration ran to — the floor the next may not reach
    /// back past, or two bursts splice into frames neither transfer sent.
    #[serde(skip)]
    h5_wave_cursor: u64,
    /// True while an H5 flush wakeup is in flight, so a burst of TXDR writes
    /// arms exactly one event chain rather than one per write.
    #[serde(skip)]
    h5_scheduled: bool,
    /// Monotonic token so a stale in-flight wakeup from a superseded arm is
    /// ignored (same shape as `Rp2040Spi::arm_seq`).
    #[serde(skip)]
    h5_arm_seq: u32,
    /// Bus cycle clock, attached by the registration choke
    /// (`add_peripheral` / `push_peripheral`). Present ⇒ this model knows "now"
    /// and can hold a burst until the wire has had time to carry it. `None`
    /// (hand-built test buses) publishes nothing, which is the honest answer:
    /// with no cycle axis there is nowhere to place a waveform.
    #[serde(skip)]
    h5_clock: Option<crate::CycleClock>,

    #[serde(skip)]
    pub attached_devices: Vec<Box<dyn SpiDevice>>,
    /// A serial-audio device on this block, when I2SCTRL.EN puts it in I2S
    /// mode. Separate from `attached_devices` because the unit is a 32-bit
    /// channel slot, not a byte -- see `I2sDevice`.
    #[serde(skip)]
    pub i2s_device: Option<Box<dyn crate::peripherals::device::I2sDevice>>,
    /// EFR32 only: "my CLK or TX currently reaches a pad", published by
    /// `GPIO_USARTROUTE`.
    ///
    /// ⚠️ WITHOUT THIS THE TWIN LIES. On Series 2 a USART's signals reach NO
    /// pin until the route registers are written, so firmware that skips them
    /// drives nothing on a real board — and this model used to clock its
    /// attached device anyway. `None` means no route block is wired (every
    /// other chip family), and the gate is then open, which keeps those
    /// families exactly as they were.
    #[serde(skip)]
    route_gate: Option<crate::peripherals::efr32::usart_route::RouteGate>,
    /// Last sampled active-low GPIO CS level for each attached device.
    #[serde(skip)]
    selected_devices: Vec<bool>,

    // ── World `gpio_net` mode (see `spi_net`) ───────────────────────────────
    /// Slave shift state between net edges.
    #[serde(skip)]
    net_slave: net::NetSlave,
    /// Master: MISO bits sampled off the net for the frame on the wire.
    #[serde(skip)]
    net_rx: u16,
    /// An interrupt a net edge raised, not yet delivered.
    #[serde(skip)]
    net_irq_pending: bool,
    /// The delivery event for `net_irq_pending` is armed.
    #[serde(skip)]
    net_irq_scheduled: bool,
    /// Frames a net-mode slave has received.
    #[serde(skip)]
    net_frames: u64,
    /// Master, NSS pulse mode: NSS is high (after a frame, or in the pulse
    /// between two).
    #[serde(skip)]
    net_nss_high: bool,

    // ── STM32 DMA handshake (CR2.TXDMAEN / RXDMAEN) ─────────────────────────
    //
    // A request line is a level (TXE / RXNE with the enable bit), and the DMA
    // answers each request with one DR access. The model raises a request as
    // a `dma_signals` id ([`crate::DMA_SIGNAL_TX`] / `RX`) and then holds it
    // OUTSTANDING until the DR access that answers it: a TX request until a
    // DR write, an RX request until a DR read. That is the request/acknowledge
    // pairing of the STM32 DMA (RM0444 §11.4.3): without it the line, still
    // high in the cycles before the DMA's item lands, would be counted again
    // and the DMA would push two items for one free slot.
    /// A TX request is outstanding (cleared by a DR write).
    #[serde(skip)]
    dma_tx_req: bool,
    /// An RX request is outstanding (cleared by a DR read, a `&self` access).
    #[serde(skip)]
    dma_rx_req: Cell<bool>,
    /// The DMA's end-of-transfer notice: the DR write now arriving is the
    /// transfer's last (`LDMA_TX`).
    #[serde(skip)]
    dma_tx_last: bool,
    /// The next RX DMA read is the transfer's last (`LDMA_RX`): with FRXTH=0
    /// its request is raised for a single byte.
    #[serde(skip)]
    dma_rx_last: bool,
    /// The [`SPI_DMA_TOKEN`] chain is armed.
    #[serde(skip)]
    dma_scheduled: bool,
    /// The limited SPI3 personality (RM0456 Table 696): DSIZE <= 15,
    /// FTHLV 0..3, TSIZE <= 0x3FF and no CTSIZE.
    #[serde(default)]
    limited_features: bool,
    /// H5 "SPI v3" receive queue. The IP completes a frame on the TXDR
    /// access rather than on a wire timeline, so a full-duplex DMA master
    /// can outrun a one-deep RXDR; silicon has a FIFO there for exactly this
    /// reason. RXDR reads pop it (RM0456 §68.5.10).
    #[serde(skip)]
    h5_rx_fifo: std::cell::RefCell<std::collections::VecDeque<(u32, u8)>>,
    /// H5 "SPI v3" transmit queue. HAL_SPI_TransmitReceive_IT pre-fills TXDR
    /// BEFORE setting CSTART, so a one-shot "write = transmit" model would
    /// drop every pre-filled frame. Frames written while stopped wait here and
    /// are shifted on CSTART.
    #[serde(skip)]
    h5_tx_fifo: std::cell::RefCell<std::collections::VecDeque<u32>>,
    /// The RX frame currently being read out of RXDR, one byte lane at a time
    /// (the bus is byte-granular; a 16-bit frame is two reads).
    #[serde(skip)]
    h5_rx_cur: std::cell::Cell<Option<(u32, u8)>>,
}

impl core::fmt::Debug for Spi {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Spi")
            .field("regs", &self.regs)
            .field("frame", &self.frame)
            .field("tx_queue_len", &self.tx_queue.len())
            .field("loopback", &self.loopback)
            .field("attached_devices", &self.attached_devices.len())
            .finish()
    }
}

impl Spi {
    pub(crate) fn has_external_bus_device(&self) -> bool {
        self.attached_devices
            .iter()
            .any(|device| device.needs_external_bus_poll())
    }

    pub(crate) fn poll_external_bus_devices(&mut self) {
        for device in &mut self.attached_devices {
            device.poll_external_bus();
        }
    }
    pub fn new() -> Self {
        Self::new_with_layout(SpiRegisterLayout::Stm32)
    }

    pub fn new_with_layout(layout: SpiRegisterLayout) -> Self {
        Self::new_with_layout_cr2(layout, 0x0000_00E7)
    }

    /// Like [`new_with_layout`] but with an explicit classic-SPI CR2 writable
    /// mask — the per-part delta (F1 `0xE7`, F4 `0xF7` for the FRF bit).
    pub fn new_with_layout_cr2(layout: SpiRegisterLayout, cr2_mask: u32) -> Self {
        let rx_fifo = matches!(layout, SpiRegisterLayout::Stm32Fifo);
        let regs = match layout {
            // CR2 reset is silicon-verified over SWD:
            //   FIFO SPI (L4/F7): CR2 = 0x0700 (DS=0b0111 8-bit + FRXTH).
            //   Classic SPI (F1/F4/L0): CR2 = 0x0000 (no DS field).
            SpiRegisterLayout::Stm32 => SpiRegs::Stm32(Stm32SpiRegs {
                fifo: false,
                cr2: 0x0000,
                sr: 0x0002, // TXE = 1
                ..Default::default()
            }),
            SpiRegisterLayout::Stm32Fifo => SpiRegs::Stm32(Stm32SpiRegs {
                fifo: true,
                cr2: 0x0700,
                sr: 0x0002,
                ..Default::default()
            }),
            SpiRegisterLayout::Stm32H5 => SpiRegs::Stm32H5(Stm32H5SpiRegs::reset()),
            SpiRegisterLayout::Nrf52Spim => SpiRegs::Nrf52(Nrf52SpiRegs::new_nrf52()),
            SpiRegisterLayout::Nrf54lSpim => SpiRegs::Nrf52(Nrf52SpiRegs::new_nrf54l()),
            SpiRegisterLayout::KinetisDspi => SpiRegs::KinetisDspi(KinetisDspiRegs::default()),
            SpiRegisterLayout::Efr32s2Usart => SpiRegs::Efr32s2Usart(Efr32s2SpiRegs::default()),
        };
        // ⚠️ The EFR32 line cell is created EAGERLY, unlike every other layout
        // here. The others get theirs when GPIO routing calls
        // `line_levels_arc()`, which is enough for a PAD probe — nothing is
        // visible on a pad until one is muxed. The WIRE probe exists precisely
        // so a bus can be measured with NO routing, and it can only resolve a
        // cell that already exists: lazy here means `resolve_wire_source`
        // succeeds and `logic_watch` hands back `None`.
        let efr32 = matches!(regs, SpiRegs::Efr32s2Usart(_));
        let mut spi = Self {
            regs,
            rx_fifo,
            cr2_mask,
            ..Default::default()
        };
        if efr32 {
            let _ = spi.line_levels_arc();
        }
        spi
    }

    pub fn set_loopback(&mut self, on: bool) {
        self.loopback = on;
    }

    /// Override the classic-SPI CR1 writable mask (default fully writable). Used
    /// by chips like F407 whose silicon does not latch CR1 bit 12 (CRCNEXT).
    pub fn set_cr1_mask(&mut self, mask: u16) {
        self.cr1_mask = Some(mask);
    }

    /// Raw device push — does NOT wrap for tracing. The only caller is the bus
    /// choke point [`crate::bus::SystemBus::attach_spi_device`], which wraps the
    /// device first; nothing else should attach directly (that would bypass the
    /// universal bus trace).
    pub(crate) fn push_device(&mut self, device: Box<dyn SpiDevice>) {
        // Resolve the opt-in ONCE, here, so the per-frame path only tests an
        // `Option` and the default path never calls `sampling()` again.
        if self.edge_slave.is_none() {
            if let SpiSampling::Edge { cpol, cpha } = device.sampling() {
                self.edge_slave = Some((cpol, cpha));
            }
        }
        self.attached_devices.push(device);
        self.selected_devices.push(false);
    }

    fn is_nrf(&self) -> bool {
        matches!(self.regs, SpiRegs::Nrf52(_))
    }

    /// `true` when this instance carries the Nordic SPIM register file — the
    /// layout whose pads are selected by `PSEL` rather than by an AF nibble.
    /// Read by `SystemBus::wire_nrf52_pads` to pick the right wiring seam.
    pub(crate) fn is_nrf_wire_layout(&self) -> bool {
        self.is_nrf()
    }

    /// Join the chip's pin-claim table so `PSEL.SCK`/`PSEL.MOSI` decide which
    /// pads read this SPIM's wire. Config-build time only; no-op on any layout
    /// but the Nordic one.
    pub(crate) fn install_nrf_pin_claims(
        &mut self,
        claims: &std::sync::Arc<crate::peripherals::nrf52::pin_select::NrfPinClaims>,
        sck_token: u32,
        mosi_token: u32,
        miso_token: u32,
    ) {
        if !self.is_nrf() {
            return;
        }
        self.nrf52_claim_sck.install(claims.clone(), sck_token);
        self.nrf52_claim_mosi.install(claims.clone(), mosi_token);
        self.nrf52_claim_miso.install(claims.clone(), miso_token);
        // MISO is an input to the SPIM: a pad routed to it drives nothing.
        self.line_levels_arc().pad_lines().set_mode(
            SpiSignal::Miso as usize,
            crate::peripherals::pad_lines::LineDrive::Input,
        );
        self.sync_nrf_pin_claims();
    }

    /// SPIM3: also claim the pads `PSEL.CSN` and `PSELDCX` name, which this
    /// instance drives itself (see `spi_nrf_net`). Config-build time only,
    /// after [`Self::install_nrf_pin_claims`].
    pub(crate) fn install_nrf_hw_select_claims(
        &mut self,
        claims: &std::sync::Arc<crate::peripherals::nrf52::pin_select::NrfPinClaims>,
        csn_token: u32,
        dcx_token: u32,
    ) {
        if !self.is_nrf() {
            return;
        }
        self.nrf52_hw_select = true;
        self.nrf52_claim_csn.install(claims.clone(), csn_token);
        self.nrf52_claim_dcx.install(claims.clone(), dcx_token);
        // Driven by the net engine only, and only while their claim is live.
        let lines = self.line_levels_arc();
        for line in [SpiSignal::Nss as usize, NRF_SPIM_LINE_DCX] {
            lines
                .pad_lines()
                .set_mode(line, crate::peripherals::pad_lines::LineDrive::Input);
        }
        self.sync_nrf_pin_claims();
    }

    /// Republish both claims from the live registers, after any write that can
    /// move a pad — `PSEL.SCK`, `PSEL.MOSI` and `ENABLE`.
    ///
    /// The gate is `ENABLE == 7` (SPIM), not merely "nonzero": SPIM0/TWIM0
    /// share one MMIO window and `ENABLE` picks which of them owns the pins
    /// (nRF52840 PS v1.11 §6.25.6.17, p733 — Enabled = 7). With `ENABLE = 6`
    /// the TWIM half is driving, and this half must not claim its pads.
    fn sync_nrf_pin_claims(&mut self) {
        let (enable, psel_sck, psel_mosi, psel_miso) = match &self.regs {
            SpiRegs::Nrf52(r) => (r.enable & 0xF, r.psel_sck, r.psel_mosi, r.psel_miso),
            _ => return,
        };
        let live = enable == NRF52_ENABLE_SPIM;
        self.nrf52_claim_sck.update(psel_sck, live);
        self.nrf52_claim_mosi.update(psel_mosi, live);
        self.nrf52_claim_miso.update(psel_miso, live);
        if self.nrf52_hw_select {
            let (psel_csn, psel_dcx) = match &self.regs {
                SpiRegs::Nrf52(r) => (r.psel_csn, r.psel_dcx),
                _ => return,
            };
            self.nrf52_claim_csn.update(psel_csn, live);
            self.nrf52_claim_dcx.update(psel_dcx, live);
        }
    }

    /// One SCK period in engine cycles, from `FREQUENCY`.
    ///
    /// `FREQUENCY` is an ENUMERATED field, not a divisor: nRF52840 PS v1.11
    /// §6.25.6.19 (p734) lists K125 = 0x02000000 … M8 = 0x80000000, and then
    /// breaks any formula you might have inferred with M16 = 0x0A000000 and
    /// M32 = 0x14000000. So the table is transcribed, and an unrecognised value
    /// falls back to the RESET value (K250) rather than being arithmetic'd into
    /// a bit rate the part cannot produce.
    fn nrf52_sck_bit_cycles(&self) -> u64 {
        const CORE_HZ: u64 = 64_000_000;
        let frequency = match &self.regs {
            // nRF54L: PRESCALER.DIVISOR is a genuine core-clock divisor, not an
            // enumeration (SVD GLOBAL_SPIM00.PRESCALER, "Core clock to SCK
            // divisor", reset 0x40). SPIM00's source is `hfpll` at 128 MHz and
            // the engine cycle IS a 128 MHz core cycle, so cycles-per-SCK-bit
            // is the divisor itself — no rate arithmetic to get wrong. The
            // floor of 2 matches the part's own minimum divisor.
            SpiRegs::Nrf52(r) if r.map == NrfSpimMap::Nrf54l => {
                return u64::from(r.prescaler & 0xFFFF).max(2)
            }
            SpiRegs::Nrf52(r) => r.frequency,
            _ => return 2,
        };
        let bps: u64 = match frequency {
            0x0200_0000 => 125_000,
            0x0400_0000 => 250_000,
            0x0800_0000 => 500_000,
            0x1000_0000 => 1_000_000,
            0x2000_0000 => 2_000_000,
            0x4000_0000 => 4_000_000,
            0x8000_0000 => 8_000_000,
            0x0A00_0000 => 16_000_000,
            0x1400_0000 => 32_000_000,
            // Reset value, and the honest answer for anything unlisted.
            _ => 250_000,
        };
        (CORE_HZ / bps).max(2)
    }

    /// Frame shape from `CONFIG` (nRF52840 PS v1.11 §6.25.6.22, p737): bit 0
    /// ORDER (0 = MsbFirst), bit 1 CPHA (0 = Leading), bit 2 CPOL
    /// (0 = ActiveHigh). SPIM has no word-length field — frames are 8 bits.
    ///
    /// Returns `None` when `ORDER` selects LsbFirst. [`SpiNarrator`] draws
    /// MSB-first only, and narrating an LSB-first transfer through it would
    /// produce a trace that looks entirely plausible and decodes to the
    /// bit-reversed byte — the single most damaging way to get a waveform
    /// wrong. A gap is honest; a reversed byte is not. LsbFirst joins the
    /// narrator when the narrator can draw it.
    /// Frame shape for the EFR32 Series-2 USART in synchronous mode, read from
    /// the live `CTRL`.
    ///
    /// Returns `None` when `MSBF` is clear: [`SpiNarrator`] draws MSB-first
    /// only, and a bit order it cannot draw publishes nothing rather than a
    /// waveform that decodes to a different byte than the one that moved.
    fn efr32_framing(&self) -> Option<SpiFraming> {
        let ctrl = match &self.regs {
            SpiRegs::Efr32s2Usart(r) => r.ctrl,
            _ => return None,
        };
        if ctrl & EFR_USART_CTRL_MSBF == 0 {
            return None;
        }
        Some(SpiFraming {
            cpol: ctrl & EFR_USART_CTRL_CLKPOL != 0,
            cpha: ctrl & EFR_USART_CTRL_CLKPHA != 0,
            // `FRAME.DATABITS` is modelled as the 8-bit reset value; this
            // controller's transfer path is byte-wide, so a wider programmed
            // frame would not have moved wider data either.
            bits: 8,
        })
    }

    /// Core cycles one SCK bit occupies, from `CLKDIV`.
    ///
    /// `f_SCK = f_PCLK / (2 * (1 + CLKDIV/256))` — the reference-manual form
    /// emlib's `USART_BaudrateSyncSet` inverts. CLKDIV's fractional field lives
    /// at bits 3..20, so the integer part of `CLKDIV/256` is `(clkdiv >> 8)`.
    ///
    /// ⚠️ The core:PCLK ratio is a CONSTANT here, exactly as the STM32L4 model
    /// does it (`CORE_PER_KCLK`). On BRD2709A the descriptor declares
    /// `cpu_hz` 78 MHz against a 19 MHz peripheral band, so 4. This governs
    /// how WIDE the published waveform is on the timeline, never what it
    /// decodes to — the bytes are the bytes the transfer moved.
    fn efr32_sck_bit_cycles(&self) -> u64 {
        let clkdiv = match &self.regs {
            SpiRegs::Efr32s2Usart(r) => u64::from(r.clkdiv),
            _ => return 2,
        };
        const CORE_PER_PCLK: u64 = 4;
        let pclk_per_bit = 2 * (1 + (clkdiv >> 8));
        (pclk_per_bit * CORE_PER_PCLK).max(2)
    }

    /// Publish one completed EFR32 byte onto the routed pads.
    ///
    /// ⚠️ Narrated PER BYTE, not per burst — the opposite of the nRF and H5
    /// paths above, and for a reason that is a property of this controller.
    /// Those move a whole DMA buffer inside one register write, so their frames
    /// have no distinct moments to be placed at and are emitted as one run. A
    /// Series-2 USART is written a byte at a time by firmware, so every byte
    /// really does have its own moment, and the sketch's own delays between
    /// them are real wire idle. Emitting per byte puts each frame where it
    /// happened instead of collapsing a paced transfer into one block.
    /// Cycles the held EFR32 burst still needs before the wire could have
    /// carried it. 0 when it is due now or nothing is held.
    fn efr32_wire_pending_cycles(&self) -> u64 {
        if self.efr32_wire_bytes.is_empty() {
            return 0;
        }
        let Some(lines) = self.lines.as_ref() else {
            return 0;
        };
        let Some(framing) = self.efr32_framing() else {
            return 0;
        };
        let Some(now) = lines.pad_lines().tap_clock() else {
            return 0;
        };
        let duration =
            self.efr32_wire_bytes.len() as u64 * framing.frame_bits() * self.efr32_sck_bit_cycles();
        self.efr32_wave_cursor
            .saturating_add(duration)
            .saturating_sub(now)
    }

    fn efr32_wire_push(&mut self, mosi: u8) {
        if self.lines.is_none() || self.efr32_framing().is_none() {
            return;
        }
        self.efr32_wire_bytes.push(mosi);
        self.efr32_wire_flush();
    }

    /// Publish whatever is buffered, if the timeline has room for it.
    ///
    /// `emit_between` refuses to reach back past the cursor — the cycle the
    /// last flush already drew to — so a burst written back-to-back stays
    /// buffered until enough cycles have passed to draw it, and then goes out
    /// as one contiguous run. That is the difference between a trace that
    /// decodes to the bytes that moved and three frames stacked on one cycle.
    fn efr32_wire_flush(&mut self) {
        if self.efr32_wire_bytes.is_empty() {
            return;
        }
        let Some(lines) = self.lines.clone() else {
            self.efr32_wire_bytes.clear();
            return;
        };
        let Some(framing) = self.efr32_framing() else {
            return;
        };
        let pads = lines.pad_lines();
        // ⚠️ `None` means NO capture tap is installed, i.e. there is no
        // timeline for a waveform to land on — not "cycle zero". Holding a run
        // back for a timeline that will never open is what made this buffer
        // grow forever; see `EFR32_WIRE_BYTE_CAP`.
        let tap_clock = pads.tap_clock();
        let now = tap_clock.unwrap_or(0);
        let mut wave = SpiNarrator::with_lines(
            SpiSignal::Sck as usize,
            SpiSignal::Mosi as usize,
            // Series 2 can route a hardware CS, but the Arduino path drives it
            // from a plain GPIO and no chip config maps the hardware one — so
            // there is no CS wire to narrate rather than an assumed one.
            None,
            &[
                pads.level(SpiSignal::Sck as usize),
                pads.level(SpiSignal::Mosi as usize),
                pads.level(SpiSignal::Miso as usize),
            ],
            self.efr32_sck_bit_cycles(),
        );
        for &byte in &self.efr32_wire_bytes {
            wave.frame(u16::from(byte), framing);
        }
        match wave.emit_between(pads, self.efr32_wave_cursor, now) {
            // No room on the timeline yet: hold the bytes and try again once
            // the machine has stepped. Drawing now would compress the run onto
            // a single cycle.
            //
            // ⚠️ HOLD ONLY WHILE HOLDING CAN STILL PAY OFF. Two guards, and
            // neither is optional:
            //
            //   * `tap_clock.is_some()` — with no capture tap there is no
            //     timeline, `now` is 0 for the whole run, and the window
            //     `cursor..now` is empty FOREVER. Every byte would be held and
            //     every retry would re-narrate all of them.
            //   * the cap — a run past `EFR32_WIRE_BYTE_CAP` has more edges
            //     than any analyzer window shows, so holding it buys a reader
            //     nothing while costing O(held) on every push and on every
            //     one-cycle `on_event` retry.
            //
            // Falling through publishes the LEVELS the run ended on (that is
            // what `LevelsOnly` already applied) and drops the waveform — the
            // same trade `nrf52_wire_flush` makes at `NRF52_WIRE_BYTE_CAP`.
            NarrationFit::LevelsOnly { .. }
                if tap_clock.is_some() && self.efr32_wire_bytes.len() < EFR32_WIRE_BYTE_CAP => {}
            _ => {
                self.efr32_wave_cursor = now;
                self.efr32_wire_bytes.clear();
                self.efr32_scheduled = false;
            }
        }
    }

    fn nrf52_framing(&self) -> Option<SpiFraming> {
        let config = match &self.regs {
            SpiRegs::Nrf52(r) => r.config,
            _ => return None,
        };
        if config & NRF52_CONFIG_ORDER_LSB != 0 {
            return None;
        }
        Some(SpiFraming {
            cpol: config & NRF52_CONFIG_CPOL != 0,
            cpha: config & NRF52_CONFIG_CPHA != 0,
            bits: 8,
        })
    }

    /// Publish a completed EasyDMA transfer's waveform onto the claimed pads.
    ///
    /// The transfer moves its whole buffer inside one `do_nrf52_easydma` call,
    /// so the burst is narrated as ONE contiguous run ending at the present
    /// cycle — the same arrangement as every other transaction-level narrator
    /// here, and forced by the same constraint: `LogicCapture::ingest_push`
    /// accepts stamps in the PAST only, so a byte that has not yet had time to
    /// clock out has nowhere on the timeline to go.
    fn nrf52_wire_flush(&mut self, mosi: &[u8]) {
        if mosi.is_empty() {
            return;
        }
        let Some(lines) = self.lines.clone() else {
            return;
        };
        let Some(framing) = self.nrf52_framing() else {
            return;
        };
        if mosi.len() > NRF52_WIRE_BYTE_CAP {
            // TXD.MAXCNT is 16 bits — one transfer can be 64 KiB, and a
            // full-frame display flush really is kilobytes. Drawing that many
            // edges would allocate for a waveform no analyzer window can show,
            // and truncating would decode to a transfer nobody performed.
            return;
        }
        let pads = lines.pad_lines();
        let mut wave = SpiNarrator::with_lines(
            SpiSignal::Sck as usize,
            SpiSignal::Mosi as usize,
            // SPIM0/1/2 have no hardware CSN — firmware drives chip select from
            // a plain GPIO — so there is no CSN wire to narrate. (SPIM3 does;
            // no chip config maps it, so it is out of scope rather than
            // assumed.) PS v1.11 §6.25.3, p726.
            None,
            &[
                pads.level(SpiSignal::Sck as usize),
                pads.level(SpiSignal::Mosi as usize),
                pads.level(SpiSignal::Miso as usize),
            ],
            self.nrf52_sck_bit_cycles(),
        );
        for &byte in mosi {
            wave.frame(u16::from(byte), framing);
        }
        let now = pads.tap_clock().unwrap_or(0);
        // A transfer early in a run has less history behind it than the
        // waveform needs; the narrator compresses to fit rather than emitting a
        // spike. The bytes still decode; only the timebase gives.
        let _fit = wave.emit_ending_at(pads, now);
    }

    /// Frame shape for the H5 "SPI v3" IP, read from the live CFG1/CFG2.
    ///
    /// Field positions are taken from this repo's SVD-derived register schema
    /// `configs/peripherals/stm32h563/spi1.yaml` (the same document the
    /// debugger inspects), NOT recalled: `SPI_CFG1.DSIZE [4:0]`, and
    /// `SPI_CFG2.LSBFRST [23]`, `CPHA [24]`, `CPOL [25]`.
    ///
    /// `DSIZE` is frame-width-minus-one, so `bits = DSIZE + 1`.
    /// [`SpiFraming::frame_bits`] clamps to 4..=16; the H5 supports up to 32-bit
    /// frames, so a wider programmed frame narrates as its low 16 bits — the
    /// same honest limit the byte-level device contract already carries.
    ///
    /// Returns `None` when `LSBFRST` selects LSB-first. [`SpiNarrator`] draws
    /// MSB-first only, and narrating an LSB-first transfer through it would
    /// produce a trace that looks entirely plausible and decodes to the
    /// bit-reversed word — the single most damaging way to get a waveform
    /// wrong. A gap is honest; a reversed word is not. Same refusal, and the
    /// same reason, as [`Self::nrf52_framing`].
    fn h5_framing(&self) -> Option<SpiFraming> {
        let (cfg1, cfg2) = match &self.regs {
            SpiRegs::Stm32H5(r) => (r.cfg1, r.cfg2),
            _ => return None,
        };
        if cfg2 & H5_CFG2_LSBFRST != 0 {
            return None;
        }
        Some(SpiFraming {
            cpol: cfg2 & H5_CFG2_CPOL != 0,
            cpha: cfg2 & H5_CFG2_CPHA != 0,
            bits: ((cfg1 & H5_CFG1_DSIZE) as u8)
                .saturating_add(1)
                .clamp(4, 16),
        })
    }

    /// Engine cycles in one SCK period, from `SPI_CFG1.MBR [30:28]` ("master
    /// baud rate prescaler setting", per this repo's SVD-derived schema
    /// `configs/peripherals/stm32h563/spi1.yaml`).
    ///
    /// ⚠️ SPEC-DERIVED, NOT SILICON-PINNED. The divider is the standard STM32
    /// SPI v3 `spi_ker_ck / 2^(MBR+1)`. RM0481 is NOT in this checkout's
    /// datasheet corpus (`labwired_datasheet` holds DATASHEETS only — DS14258
    /// for the H563, DS13312 for the H735 — no reference manual), so the
    /// exponent could not be read from a citable page. This is the SAME class
    /// of divergence `configs/chips/stm32h563.yaml` already declares for this
    /// peripheral: "the bench part had no SPI kernel clock, so the sim's
    /// always-clocked TX engine is spec-derived, not silicon-pinned".
    ///
    /// The consequence is bounded and worth stating plainly: the frames, their
    /// bit ORDER, widths, CPOL/CPHA and the sampling edge are all real, so an
    /// independent decoder recovers exactly the words the model shifted. It is
    /// the measured bit RATE that carries the spec-derived assumption.
    ///
    /// Kernel-clock ticks are used as engine cycles, the same assumption
    /// `Rp2040Spi::bit_time_cycles` documents for `clk_peri`.
    fn h5_bit_time_cycles(&self) -> Option<u64> {
        let cfg1 = match &self.regs {
            SpiRegs::Stm32H5(r) => r.cfg1,
            _ => return None,
        };
        let mbr = (cfg1 & H5_CFG1_MBR) >> H5_CFG1_MBR_SHIFT;
        Some(1u64 << (u64::from(mbr) + 1))
    }

    /// Queue one transmitted frame for narration. Buffered, not published — see
    /// [`Self::h5_wire_flush`]. No routed pads ⇒ nothing to narrate, and the
    /// call costs one branch, which is every H5 lab that never clipped a probe
    /// to an SPI pad.
    fn h5_wire_push(&mut self, word: u16) {
        if self.lines.is_none() {
            return;
        }
        let (Some(framing), Some(bit_time)) = (self.h5_framing(), self.h5_bit_time_cycles()) else {
            return;
        };
        // A rate change mid-burst: publish what is held at the rate it was
        // shifted at, THEN start a new burst. Repainting held frames at the new
        // rate would report a bit period no transfer ever used.
        if !self.h5_wire_words.is_empty() && bit_time != self.h5_wire_bit_time {
            self.h5_wire_flush(true);
        }
        if self.h5_wire_words.is_empty() {
            self.h5_wire_bit_time = bit_time;
        }
        self.h5_wire_words.push((word, framing));
    }

    /// Cycles until the buffered H5 burst has had its wire time — 0 when it is
    /// due now, when nothing is buffered, or when the burst has hit the cap and
    /// must be published compressed rather than held any longer.
    ///
    /// Both the pacing test and the scheduler deadline, computed the ONE way,
    /// so a wakeup can never land before the burst is publishable (a wasted
    /// wakeup) or after it (a late trace).
    fn h5_wire_ready_in(&self) -> u64 {
        if self.h5_wire_words.is_empty() || self.h5_wire_words.len() >= H5_WIRE_BURST_CAP {
            return 0;
        }
        self.h5_wire_pending_cycles()
    }

    /// Cycles the buffered burst still needs before the wire could have carried
    /// it — the SAME arithmetic as [`Self::h5_wire_ready_in`] but WITHOUT the
    /// burst-cap short-circuit.
    ///
    /// # Why the cap must not shortcut this
    ///
    /// A burst that has hit the cap is published `force`d, and a forced publish
    /// can still come back [`NarrationFit::LevelsOnly`]: the waveform has more
    /// transitions than the cycle window `now - wave_cursor` has room for, so
    /// nothing is drawn and the frames are (correctly) kept.
    ///
    /// Rescheduling that retry off `h5_wire_ready_in` would ask for `0`,
    /// clamped to 1 — a retry EVERY CYCLE, each one rebuilding a 256-frame
    /// narration plan (~4900 edges) only to fail again until enough cycles
    /// accumulate. On the display-heavy H735 telematics lab
    /// (`examples/stm32h735-smoke`, 50M steps driving a TFT over SPI) that is
    /// thousands of full plan rebuilds per burst, and it made the board's own
    /// onboarding smoke crawl.
    ///
    /// Asking instead for the cycles the burst genuinely still needs turns that
    /// spin into ONE well-timed wakeup, which is the whole point of pacing a
    /// narration against the wire.
    fn h5_wire_pending_cycles(&self) -> u64 {
        if self.h5_wire_words.is_empty() {
            return 0;
        }
        let Some(clock) = &self.h5_clock else {
            return 0;
        };
        let duration: u64 = self
            .h5_wire_words
            .iter()
            .map(|(_, framing)| framing.frame_bits() * self.h5_wire_bit_time)
            .sum();
        self.h5_wave_cursor
            .saturating_add(duration)
            .saturating_sub(clock.now())
    }

    /// Publish the buffered H5 frames onto the routed pads, once the wire has
    /// had time to carry them.
    ///
    /// The transaction-level model completes a frame inside the TXDR write and
    /// leaves TXP set, so a HAL transmit loop hands this model a whole buffer
    /// within a few dozen cycles. The WIRE cannot do that, and the capture layer
    /// (`LogicTap::push_at` → `LogicCapture::ingest_push`) accepts stamps in the
    /// PAST only and keeps a single level per channel per cycle — there is
    /// simply nowhere to put a frame that has not yet had time to cross. So the
    /// burst accumulates and is narrated as one waveform ending at the present
    /// cycle, exactly as every other narrator here publishes.
    ///
    /// `force` publishes regardless (the cap and rate-change paths). The burst
    /// is then compressed: the words stay readable, the timebase does not.
    fn h5_wire_flush(&mut self, force: bool) {
        if self.h5_wire_words.is_empty() {
            return;
        }
        let (Some(levels), Some(clock)) = (self.lines.clone(), self.h5_clock.clone()) else {
            // No cycle axis (hand-built bus) ⇒ nowhere to place a waveform.
            self.h5_wire_words.clear();
            return;
        };
        let now = clock.now();
        if !force && self.h5_wire_ready_in() > 0 {
            return;
        }
        let pads = levels.pad_lines();
        let mut wave = SpiNarrator::with_lines(
            SpiSignal::Sck as usize,
            SpiSignal::Mosi as usize,
            // ── CHIP-SELECT FRAMING: deliberately NOT narrated ──────────────
            // `SpiLineLevels` carries exactly three lines (SCK, MOSI, MISO) and
            // the H5 AF pad tables in `bus::attach` route only those three, so
            // there is no CS wire in this model to draw on.
            //
            // That is also the conservative answer to a question this checkout
            // CANNOT settle: whether SPI v3 pulses NSS between back-to-back
            // frames or holds it low. The PL022 rule the RP2040 narrator cites
            // (datasheet §4.4.3 — SPH=0 pulses, SPH=1 holds) is a PL022 rule and
            // does NOT carry to ST's IP, whose framing is governed by
            // CFG2.SSOE/SSOM (bits [29]/[30] in this repo's SVD schema) rather
            // than by CPHA at all. Settling it needs RM0481, which is not in the
            // corpus. Narrating no CS cannot merge two frames into one or split
            // one into two — an absent line is honest where a guessed one is
            // not.
            None,
            &[
                pads.level(SpiSignal::Sck as usize),
                pads.level(SpiSignal::Mosi as usize),
                pads.level(SpiSignal::Miso as usize),
            ],
            self.h5_wire_bit_time,
        );
        for &(word, framing) in &self.h5_wire_words {
            wave.frame(word, framing);
        }
        if let NarrationFit::LevelsOnly { .. } = wave.emit_between(pads, self.h5_wave_cursor, now) {
            // Fewer cycles exist than the waveform has transitions, so nothing
            // was drawn. Keep the words and the cursor: `now` only grows, so a
            // later wakeup will have the room. Clearing here would delete frames
            // that really crossed the bus and advance the cursor past cycles
            // nothing ever painted — silent, unrecoverable loss.
            return;
        }
        self.h5_wave_cursor = now;
        self.h5_wire_words.clear();
    }

    /// STM32 register write with transfer-engine side effects. Only called on
    /// the STM32 variant.
    fn write_stm32_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                // Classic SPI CR1 is fully writable incl. CRCNEXT (bit 12) on
                // F103/L0/L476 (silicon-confirmed, CR1 reads back 0xFFFF). F407
                // silicon does NOT latch CRCNEXT — writing 0xFFFF reads back
                // 0xEFFF — so its chip config sets `cr1_mask: 0xEFFF`. The FIFO
                // variant (L4/F7/H5) has a different CR1 bit map; both store the
                // (masked) written value verbatim.
                let cr1_mask = self.cr1_mask.unwrap_or(0xFFFF);
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    r.cr1 = value & cr1_mask;
                }
                // A CPOL change while the wire is idle re-drives the SCK idle
                // level (real silicon drives SCK = CPOL as soon as the pad is
                // handed to the SPI).
                if self.frame.is_none() {
                    if let Some(lines) = &self.lines {
                        let cpol = value & (1 << 1) != 0;
                        lines.set(cpol, lines.mosi(), lines.miso());
                    }
                }
                self.net_apply_drive();
            }
            0x04 => {
                // STM32L4/F7 SPI CR2: DS[3:0] (bits 11:8) select the data
                // frame size. Values below 0b0011 are reserved and hardware
                // forces them to 0b0111 (8-bit) on FIFO parts — verified on
                // NUCLEO-L476RG (CR2=0x0000 reads back 0x0700). Classic SPI
                // has no DS field; its writable mask is the per-part `cr2_mask`
                // (F1 0xE7, F4 0xF7 for the FRF bit).
                let cr2_mask = self.cr2_mask as u16;
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    if r.fifo {
                        let ds = (value >> 8) & 0xF;
                        // Bit 15 is reserved and reads 0 (RM0444 §35.7.2).
                        let value = value & 0x7FFF;
                        r.cr2 = if ds < 0b0011 {
                            (value & !0x0F00) | (0b0111 << 8)
                        } else {
                            value
                        };
                    } else {
                        r.cr2 = value & cr2_mask;
                    }
                }
                if self.rx_fifo {
                    // A new FRXTH moves the RXNE threshold at once.
                    self.fifo_sync_rxne();
                }
                // A request line that is switched off drops its request.
                if value & CR2_TXDMAEN == 0 {
                    self.dma_tx_req = false;
                }
                if value & CR2_RXDMAEN == 0 {
                    self.dma_rx_req.set(false);
                    self.dma_rx_last = false;
                }
                self.net_apply_drive();
            }
            0x08 => {
                // SR is mostly read-only; allow clearing OVR if modelled.
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    if r.fifo {
                        // FIFO port: RXNE/TXE/FRLVL/FTLVL are functions of the
                        // FIFOs, not storage; only CRCERR (rc_w0) is
                        // software-clearable here.
                        if value & 0x0010 == 0 {
                            r.sr &= !0x0010;
                        }
                        return;
                    }
                    r.sr = value & 0xFFBF;
                    // Keep clear-on-read RXNE cell in sync with any software clear.
                    self.stm32_rxne.set(r.sr & 0x0001 != 0);
                }
            }
            0x10 => {
                // CRCPR: 16-bit CRC polynomial, plain R/W (the model previously
                // dropped writes). Silicon-confirmed writable 0xFFFF on F103.
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    r.crcpr = value;
                }
            }
            0x0C => {
                // DR write goes to the TX path only. Starts a transfer iff SPE.
                // The frame does NOT complete here: the bit engine clocks it on
                // the wire over `bits × 2^(BR+1)` peripheral-clock cycles.
                let cr1 = match &self.regs {
                    SpiRegs::Stm32(r) => r.cr1,
                    _ => 0,
                };
                // Any DR write answers an outstanding TX DMA request.
                self.dma_tx_req = false;
                self.dma_tx_last = false;
                if self.net_slave_owns_dr() {
                    // A slave on a net shifts on the master's clock.
                    self.net_slave_write_dr(value);
                    return;
                }
                if (cr1 & (1 << 6)) != 0 {
                    let fifo = matches!(&self.regs, SpiRegs::Stm32(r) if r.fifo);
                    if let SpiRegs::Stm32(r) = &mut self.regs {
                        r.sr &= !0x0002; // Clear TXE
                        r.sr |= 0x0080; // Set BSY
                    }
                    if self.frame.is_some() && !fifo && !self.tx_queue.is_empty() {
                        // Classic single-buffer TX: a DR write while TXE=0
                        // overwrites the waiting byte (RM0008 — the shifting
                        // frame is unaffected).
                        *self.tx_queue.back_mut().unwrap() = value;
                    } else if fifo && !self.fifo_tx_has_room() {
                        // FIFO parts: the 32-bit TX FIFO is full (four 8-bit
                        // or two 16-bit frames) — the write is lost (RM0444
                        // §35.5.9; firmware must wait for TXE).
                    } else {
                        self.tx_queue.push_back(value);
                    }
                    if self.frame.is_none() {
                        self.stm32_start_next_frame();
                    }
                }
            }
            _ => {
                crate::census_reg!("spi:Spi", offset, "write");
            }
        }
    }

    /// NXP Kinetis DSPI register write with transfer-engine side effects. Only
    /// called on the `KinetisDspi` variant. A PUSHR write transmits one frame
    /// (broadcast to attached devices) and raises SR.TCF; SR is write-1-to-clear
    /// for TCF/EOQF/RFDF, matching the `fsl_dspi` blocking-write poll loop.
    fn write_kinetis_dspi_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                if let SpiRegs::KinetisDspi(r) = &mut self.regs {
                    // CLR_TXF/CLR_RXF are momentary (read back 0); keep the
                    // configured MCR bits otherwise.
                    r.mcr = value & !(0x0000_0C00);
                }
            }
            0x08 => {
                if let SpiRegs::KinetisDspi(r) = &mut self.regs {
                    r.tcr = value;
                }
            }
            0x0C => {
                if let SpiRegs::KinetisDspi(r) = &mut self.regs {
                    r.ctar[0] = value;
                }
            }
            0x10 => {
                if let SpiRegs::KinetisDspi(r) = &mut self.regs {
                    r.ctar[1] = value;
                }
            }
            0x2C => {
                // SR: TCF/EOQF/RFDF are write-1-to-clear; TFFF stays asserted.
                if let SpiRegs::KinetisDspi(r) = &mut self.regs {
                    r.sr &= !(value & (DSPI_SR_TCF | DSPI_SR_EOQF | DSPI_SR_RFDF));
                }
            }
            0x30 => {
                if let SpiRegs::KinetisDspi(r) = &mut self.regs {
                    r.rser = value;
                }
            }
            0x34 => {
                // PUSHR: low 16 bits are the frame data. PCD8544 and most SPI
                // peripherals here clock 8-bit frames, so deliver the low byte.
                let mosi = (value & 0xFF) as u8;
                let mut miso: u8 = 0;
                for dev in &mut self.attached_devices {
                    let resp = dev.transfer(mosi);
                    if resp != 0 {
                        miso = resp;
                    }
                }
                if self.loopback && self.attached_devices.is_empty() {
                    miso = mosi;
                }
                if let SpiRegs::KinetisDspi(r) = &mut self.regs {
                    r.popr = miso as u32;
                    // Frame complete: raise TCF (and RFDF — a byte landed in the
                    // RX FIFO). TFFF remains set (FIFO has room).
                    r.sr |= DSPI_SR_TCF | DSPI_SR_RFDF | DSPI_SR_TFFF;
                }
            }
            _ => {
                crate::census_reg!("spi:Spi", offset, "write");
            }
        }
    }

    /// EFR32 Series-2 USART (synchronous mode) register write.
    ///
    /// A `TXDATA` write clocks ONE frame out and back: the byte goes to every
    /// attached device and the last non-zero answer lands in `RXDATA`, which is
    /// the same broadcast rule the Kinetis DSPI path uses. `emlib`'s
    /// `USART_SpiTransfer` is exactly this — write TXDATA, wait `STATUS.TXC`,
    /// read RXDATA.
    ///
    /// ⚠️ A frame is only clocked when the USART is ENABLED and in SYNC mode
    /// with TX enabled. A driver that forgot `CTRL.SYNC` or `CMD.TXEN` gets
    /// nothing, here and on the bench — this block is a UART until told
    /// otherwise, and silently transferring anyway would hide the single most
    /// common way an EFR32 SPI bring-up fails.
    fn write_efr32s2_usart_reg(&mut self, offset: u64, value: u32) {
        // Read the gating state before borrowing mutably for the transfer.
        let (enabled, sync, tx_enabled, i2s_on) = match &self.regs {
            SpiRegs::Efr32s2Usart(r) => (
                r.en & EFR_USART_EN_EN != 0,
                r.ctrl & EFR_USART_CTRL_SYNC != 0,
                r.tx_enabled,
                r.i2sctrl & EFR_USART_I2SCTRL_EN != 0,
            ),
            _ => (false, false, false, false),
        };

        match offset {
            EFR_USART_EN => {
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    r.en = value & EFR_USART_EN_EN;
                    if r.en == 0 {
                        r.rx_enabled = false;
                        r.tx_enabled = false;
                        r.master = false;
                        r.rxdatav = false;
                    }
                }
            }
            EFR_USART_CTRL => {
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    r.ctrl = value;
                }
            }
            EFR_USART_FRAME => {
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    r.frame = value;
                }
            }
            EFR_USART_TRIGCTRL => {
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    r.trigctrl = value;
                }
            }
            EFR_USART_CLKDIV => {
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    r.clkdiv = value;
                }
            }
            EFR_USART_CMD => {
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    if value & EFR_USART_CMD_RXEN != 0 {
                        r.rx_enabled = true;
                    }
                    if value & EFR_USART_CMD_RXDIS != 0 {
                        r.rx_enabled = false;
                    }
                    if value & EFR_USART_CMD_TXEN != 0 {
                        r.tx_enabled = true;
                    }
                    if value & EFR_USART_CMD_TXDIS != 0 {
                        r.tx_enabled = false;
                    }
                    if value & EFR_USART_CMD_MASTEREN != 0 {
                        r.master = true;
                    }
                    if value & EFR_USART_CMD_MASTERDIS != 0 {
                        r.master = false;
                    }
                    if value & EFR_USART_CMD_CLEARRX != 0 {
                        r.rxdatav = false;
                        r.rxdata = 0;
                    }
                }
            }
            EFR_USART_TXDATA => {
                if !(enabled && sync && tx_enabled) {
                    return;
                }
                // ⚠️ AND THE SIGNALS MUST ACTUALLY REACH A PIN. On Series 2 a
                // USART is wired to pads only through `GPIO_USARTROUTE`;
                // firmware that never writes it clocks NOTHING on a real
                // board. This model used to drive its attached device anyway,
                // which is how a silabs-arduino core that programmed no route
                // passed every simulated SPI test against a dead bus.
                //
                // TXC is still raised: the block really does shift the byte
                // out of its own shift register. What does not happen is the
                // byte reaching a device — exactly the silicon behaviour.
                if !self.efr32_reaches_a_pad() {
                    if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                        r.iflag |= EFR_USART_IF_TXC | EFR_USART_IF_TXBL;
                        r.txc = true;
                    }
                    return;
                }
                // I2S mode: a TXDATA write clocks one 32-bit CHANNEL SLOT, not
                // a byte. On this block receiving requires clocking -- RM
                // section 20.3.3.7: "the main device must generate the bus
                // clock even when it is not transmitting data ... transmit
                // data with the transmitter tristated when receiving data".
                // So a mic is read by writing TXDATA and then reading RXDATA,
                // exactly as on silicon.
                if i2s_on {
                    let (right, i2sctrl) = match &self.regs {
                        SpiRegs::Efr32s2Usart(r) => (r.i2s_right, r.i2sctrl),
                        _ => (false, 0),
                    };
                    let raw = match self.i2s_device.as_mut() {
                        Some(dev) => dev.next_slot(right),
                        // No device: the line floats to its pulldown. Zero is
                        // the honest answer, and it is what a board with the
                        // mic left unpopulated actually reads.
                        None => 0,
                    };
                    // FORMAT decides how many of the word's MSBs the USART
                    // hands back. RM section 20.3.3.9 p.629: "configuring
                    // FORMAT to using a 32-bit word with 16-bit data will make
                    // each word on the I2S bus 32-bits wide, but when receiving
                    // data through the USART, only the 16 most significant bits
                    // of each word can be read out". Dropping this would let a
                    // driver configured for 16-bit audio read 32 bits of it and
                    // still look correct.
                    let format =
                        (i2sctrl & EFR_USART_I2SCTRL_FORMAT_MASK) >> EFR_USART_I2SCTRL_FORMAT_SHIFT;
                    let data_bits: u32 = match format {
                        0 => 32,     // W32D32
                        1 | 2 => 24, // W32D24M / W32D24
                        3 | 5 => 16, // W32D16 / W16D16
                        _ => 8,      // W32D8 / W16D8 / W8D8
                    };
                    let slot = if data_bits >= 32 {
                        raw
                    } else {
                        raw & (!0u32 << (32 - data_bits))
                    };
                    if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                        r.rxdata = slot;
                        r.rxdatav = true;
                        // Stereo alternates every word; mono pulses the word
                        // clock per word and never leaves the left channel
                        // (RM section 20.3.3.8 p.629, MONO in 20.5.22 p.669).
                        if r.i2sctrl & EFR_USART_I2SCTRL_MONO == 0 {
                            r.i2s_right = !right;
                        }
                        r.iflag |= EFR_USART_IF_TXC | EFR_USART_IF_TXBL | EFR_USART_IF_RXDATAV;
                        r.txc = true;
                    }
                    return;
                }
                let mosi = (value & 0xFF) as u8;
                let mut miso: u8 = 0;
                for dev in &mut self.attached_devices {
                    let resp = dev.transfer(mosi);
                    if resp != 0 {
                        miso = resp;
                    }
                }
                if self.loopback && self.attached_devices.is_empty() {
                    miso = mosi;
                }
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    r.rxdata = miso as u32;
                    r.rxdatav = true;
                    r.iflag |= EFR_USART_IF_TXC | EFR_USART_IF_TXBL | EFR_USART_IF_RXDATAV;
                    r.txc = true;
                }
                self.efr32_wire_push(mosi);
            }
            EFR_USART_IF => {
                // Write-1-to-clear, the Series-2 convention.
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    r.iflag &= !value;
                }
            }
            EFR_USART_IEN => {
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    r.ien = value;
                }
            }
            EFR_USART_I2SCTRL => {
                if let SpiRegs::Efr32s2Usart(r) = &mut self.regs {
                    // Entering I2S mode restarts the frame on the LEFT
                    // channel. RM section 20.3.3.11 p.632: "the USART always
                    // starts transmitting on the LEFT channel after being
                    // enabled". Without this a re-enable would resume
                    // mid-frame and hand the firmware the wrong channel's
                    // sample for the rest of the run.
                    let was_on = r.i2sctrl & EFR_USART_I2SCTRL_EN != 0;
                    let now_on = value & EFR_USART_I2SCTRL_EN != 0;
                    if now_on && !was_on {
                        r.i2s_right = false;
                    }
                    r.i2sctrl = value;
                }
            }
            _ => {
                crate::census_reg!("spi:Spi", offset, "write");
            }
        }
    }

    /// Shift one frame through the H5 SPI. Full-duplex: every transmitted
    /// frame simultaneously clocks one IN. Silicon fills RXDR and raises
    /// SR.RXP whether or not a slave drives MISO — with nothing driving, the
    /// captured value is just the idle line level.
    fn h5_transmit_frame(&mut self, word: u32) {
        let (master, frame_bytes) = match &self.regs {
            SpiRegs::Stm32H5(r) => (
                r.cfg2 & H5_CFG2_MASTER != 0,
                // CFG1.DSIZE is the frame width in BITS minus one (7=8-bit,
                // 15=16-bit, 31=32-bit), not bytes.
                ((((r.cfg1 & H5_CFG1_DSIZE) as usize) + 1 + 7) / 8).clamp(1, 4),
            ),
            _ => return,
        };
        if !master {
            return;
        }
        let mut miso_word: u32 = 0;
        for lane in (0..frame_bytes).rev() {
            let mosi = ((word >> (8 * lane)) & 0xFF) as u8;
            let mut miso: u8 = 0;
            for dev in &mut self.attached_devices {
                let r = dev.transfer(mosi);
                if r != 0 {
                    miso = r;
                }
            }
            if self.loopback && self.attached_devices.is_empty() {
                miso = mosi;
            }
            miso_word |= u32::from(miso) << (8 * lane);
            // The byte just crossed the bus. Narrate it onto the routed AF
            // pads: this IP has no bit engine to drive them per cycle.
            self.h5_wire_push(u16::from(mosi));
        }
        if let SpiRegs::Stm32H5(r) = &mut self.regs {
            r.rxdr = miso_word;
            r.sr |= H5_SR_RXP;
        }
        self.h5_rx_fifo
            .borrow_mut()
            .push_back((miso_word, frame_bytes as u8));
        // Authoritative RXNE for the DMA handshake and the
        // clear-on-RXDR-read path.
        self.stm32_rxne.set(true);
        if let SpiRegs::Stm32H5(r) = &mut self.regs {
            if r.ctsize > 0 {
                r.ctsize -= 1;
                if r.ctsize == 0 {
                    // Frame count exhausted: EOT|TXC, start request HW-cleared.
                    // TSIZE=0 (endless mode) never reaches this — no EOT.
                    r.sr |= H5_SR_EOT | H5_SR_TXC;
                    r.cr1 &= !H5_CR1_CSTART;
                }
            }
        }
    }

    /// Drain every queued frame after CSTART rises.
    fn h5_drain_tx_fifo(&mut self) {
        loop {
            let started = matches!(&self.regs, SpiRegs::Stm32H5(r) if r.cr1 & H5_CR1_CSTART != 0);
            if !started {
                return;
            }
            let next = self.h5_tx_fifo.borrow_mut().pop_front();
            let Some(mosi) = next else {
                return;
            };
            self.h5_transmit_frame(mosi);
        }
    }

    /// STM32H5 ("SPI v3") register write with transfer-engine side effects.
    /// Only called on the `Stm32H5` variant. Behavioural rules pinned by
    /// silicon capture 2026-06-11 (NUCLEO-H563ZI) unless noted otherwise.
    fn write_stm32h5_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                // CR1
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    let prev = r.cr1;
                    let mut v = value & H5_CR1_WRITABLE;
                    // While the mode-fault condition stands (SR.MODF latched),
                    // setting SPE is refused: CR1 = SPE|SSI after a fault
                    // reads back 0x1000.
                    if r.sr & H5_SR_MODF != 0 {
                        v &= !H5_CR1_SPE;
                    }
                    // CSTART latches while a transfer is active: SW may only
                    // set it (and only under SPE); HW clears it at EOT
                    // (RM0481 §41.4.10).
                    let cstart = (prev & H5_CR1_CSTART != 0)
                        || (value & H5_CR1_CSTART != 0 && v & H5_CR1_SPE != 0);
                    v = (v & !H5_CR1_CSTART) | if cstart { H5_CR1_CSTART } else { 0 };
                    if prev & H5_CR1_SPE == 0 && v & H5_CR1_SPE != 0 {
                        // SPE 0→1: load CTSIZE from CR2.TSIZE; a nonzero
                        // frame count is a pending transfer, so TXC drops
                        // (SR = 0x00020002 with TSIZE=2 on the bench).
                        r.ctsize = r.cr2 & 0xFFFF;
                        if r.ctsize > 0 {
                            r.sr &= !H5_SR_TXC;
                        }
                    } else if prev & H5_CR1_SPE != 0 && v & H5_CR1_SPE == 0 {
                        // SPE 1→0: TXC comes back, CTSIZE is retained
                        // (SR = 0x00021002 on the bench) and the start
                        // request is dropped.
                        r.sr |= H5_SR_TXC;
                        v &= !H5_CR1_CSTART;
                    }
                    r.cr1 = v;
                    if prev & H5_CR1_SPE == 0 && v & H5_CR1_SPE != 0 {
                        // A fresh enable starts from empty FIFOs.
                        self.h5_tx_fifo.borrow_mut().clear();
                        self.h5_rx_fifo.borrow_mut().clear();
                        self.h5_rx_cur.set(None);
                        self.stm32_rxne.set(false);
                    }
                    let started =
                        prev & H5_CR1_CSTART == 0 && v & H5_CR1_CSTART != 0;
                    if started {
                        self.h5_drain_tx_fifo();
                    }
                }
                // Enabling the peripheral hands the pads to the SPI, and real
                // silicon immediately drives SCK to the programmed idle
                // polarity. Publishing it HERE rather than letting the first
                // narrated frame's "park at CPOL" do it is what keeps the
                // parking transition OUTSIDE the waveform: with CPOL=1 on a
                // wire resting low, that park is a genuine rising edge, and
                // inside the narration it landed among the frame's own edges —
                // an extra trailing edge that a CPHA=1 decoder counts as a
                // sampling edge and shifts the whole byte stream by one bit.
                // The classic bit engine does the same thing on a CR1 write.
                if let SpiRegs::Stm32H5(r) = &self.regs {
                    if r.cr1 & H5_CR1_SPE != 0 {
                        let cpol = r.cfg2 & H5_CFG2_CPOL != 0;
                        if let Some(lines) = &self.lines {
                            lines.set(cpol, lines.mosi(), lines.miso());
                        }
                    }
                }
            }
            0x04 => {
                // CR2: TSIZE[15:0] (write 0x10 → reads 0x10 on the bench).
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.cr2 = if self.limited_features {
                        value & 0x03FF
                    } else {
                        value & 0xFFFF
                    };
                }
            }
            0x08 => {
                // CFG1: ignored while SPE=1 (config lock); reserved bits
                // read as 0.
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    if r.cr1 & H5_CR1_SPE == 0 {
                        let mut v = value & !H5_CFG1_RESERVED;
                        if self.limited_features {
                            v = (v & !H5_CFG1_DSIZE) | (v & 0x0F);
                            v = (v & !(0xF << 5)) | ((v >> 5 & 0x3) << 5);
                        }
                        r.cfg1 = v;
                    }
                }
                // A request line switched off drops its outstanding request.
                let cfg1 = match &self.regs {
                    SpiRegs::Stm32H5(r) => r.cfg1,
                    _ => 0,
                };
                if cfg1 & H5_CFG1_TXDMAEN == 0 {
                    self.dma_tx_req = false;
                }
                if cfg1 & H5_CFG1_RXDMAEN == 0 {
                    self.dma_rx_req.set(false);
                }
            }
            0x0C => {
                // CFG2: ignored while SPE=1. A MASTER request while the
                // internal SS level is low (SSM=1 && CR1.SSI=0) mode-faults:
                // MASTER is refused and SR.MODF latches (CFG2 write
                // 0x04400000 with SSI=0 → reads 0x04000000, SR 0x1202).
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    if r.cr1 & H5_CR1_SPE == 0 {
                        let mut v = value;
                        if v & H5_CFG2_MASTER != 0
                            && v & H5_CFG2_SSM != 0
                            && r.cr1 & H5_CR1_SSI == 0
                        {
                            v &= !H5_CFG2_MASTER;
                            r.sr |= H5_SR_MODF;
                        }
                        r.cfg2 = v;
                    }
                }
            }
            0x10 => {
                // IER (write 0x209 → reads 0x209 on the bench).
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.ier = value & H5_IER_WRITABLE;
                }
            }
            0x18 => {
                // IFCR: write-1-to-clear for the clearable SR flags.
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.sr &= !(value & H5_IFCR_W1C);
                }
            }
            0x20 => {
                // TXDR. Frames written while the transfer has not started are
                // held in the TX FIFO (HAL_SPI_TransmitReceive_IT pre-fills it
                // before CSTART); once started each write shifts one frame.
                let (spe, started) = match &self.regs {
                    SpiRegs::Stm32H5(r) => (
                        r.cr1 & H5_CR1_SPE != 0,
                        r.cr1 & H5_CR1_CSTART != 0,
                    ),
                    _ => return,
                };
                if !spe {
                    return;
                }
                // This TXDR access answers an outstanding TX DMA request.
                self.dma_tx_req = false;
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    // Any enabled TXDR write fills the TxFIFO → TXTF; TXP
                    // stays set (sim TX path is bottomless).
                    r.sr |= H5_SR_TXTF;
                }
                if started {
                    self.h5_transmit_frame(value);
                } else {
                    self.h5_tx_fifo.borrow_mut().push_back(value);
                }
            }
            0x40 => {
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.crcpoly = value;
                }
            }
            0x4C => {
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.udrdr = value;
                }
            }
            0x50 => {
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    r.i2scfgr = value;
                }
            }
            // SR (0x14) is read-only (flags clear via IFCR); TXCRC/RXCRC
            // (0x44/0x48) are HW-computed and read-only.
            _ => {
                crate::census_reg!("spi:Spi", offset, "write");
            }
        }
    }
}

// ── STM32 bit engine ─────────────────────────────────────────────────────────
//
// A frame executes on the wire as a chain of fixed-level half-period segments.
// Slaves stay byte-level: the engine consults them exactly once per frame (at
// the boundary where the frame starts clocking) and the answered byte is what
// MISO carries bit-by-bit during that same frame.
impl Spi {
    /// `true` while a frame is clocking on the wire. Production code reads
    /// SR.BSY instead; tests clock the engine against this directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn transfer_active(&self) -> bool {
        self.frame.is_some()
    }

    /// `true` when this instance carries the classic/FIFO STM32 register file
    /// (the layouts the bit engine drives). The H5 "SPI v3" IP and the other
    /// vendor layouts are separate models.
    ///
    /// ⚠️ THIS PREDICATE MEANS "HAS A BIT ENGINE". It is NOT the pad-routing
    /// question — [`Self::publishes_stm32_pad_wire`] is. Do not widen this
    /// `matches!` to admit `Stm32H5(_)` to make pads route, however tempting the
    /// one-line diff looks.
    ///
    /// The reason is a collision that is invisible from this branch alone. The
    /// unmerged `feat/spi-edge-sampling` branch REPURPOSES this same predicate
    /// as its "has a bit engine" gate: in its `attach_spi_device` it refuses
    /// edge-accurate slave sampling with
    /// `if edge_sampled && !c.is_stm32_wire_layout()`, naming the refusal
    /// "STM32H5 SPIv3 / Kinetis DSPI". Widening this to include the H5 would
    /// silently flip that guard FALSE for the H5 and hand edge-accurate
    /// sampling to a controller that completes a whole frame inside one TXDR
    /// write and has no bit index to sample on — defeating a refusal that
    /// exists to keep a lab author from watching a mode-mismatch lesson fail to
    /// reproduce. The two questions genuinely differ: the H5 CAN publish a
    /// narrated wire onto pads, and CANNOT be sampled edge-accurately.
    ///
    /// On THIS branch nothing in the engine calls it any more — pad routing
    /// moved to `publishes_stm32_pad_wire` — so it is exercised only by the
    /// test that pins the two apart. It is KEPT rather than deleted because it
    /// is the gate `feat/spi-edge-sampling` refuses edge-accurate sampling
    /// with; removing it would turn that branch's merge into a silent
    /// re-resolution of the exact question this pair exists to keep separate.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn is_stm32_wire_layout(&self) -> bool {
        matches!(self.regs, SpiRegs::Stm32(_))
    }

    /// `true` when this instance can publish a real waveform onto STM32
    /// alternate-function pads — the PAD-ROUTING question, deliberately
    /// separate from [`Self::is_stm32_wire_layout`] (read the warning there).
    ///
    /// Two different mechanisms answer `true` here, which is exactly why this
    /// is its own predicate rather than a widened bit-engine test:
    /// * the classic/FIFO layouts drive [`SpiLineLevels`] per cycle from the
    ///   bit engine, and
    /// * the H5 "SPI v3" layout has no bit engine at all and instead NARRATES
    ///   the waveform its transaction-level transfer implies
    ///   ([`Self::h5_wire_flush`]).
    ///
    /// Both end up publishing into the same [`PadLines`] cell, so the GPIO pad
    /// routing is identical for both; only how the levels get there differs.
    pub(crate) fn publishes_stm32_pad_wire(&self) -> bool {
        matches!(self.regs, SpiRegs::Stm32(_) | SpiRegs::Stm32H5(_))
    }

    /// `true` for the H5/H7/U5 "SPI v3" register file, which selects the
    /// declared H5-class alternate-function pad table (H5/WBA/U5) rather than
    /// the classic/FIFO ones.
    pub(crate) fn is_h5_wire_layout(&self) -> bool {
        matches!(self.regs, SpiRegs::Stm32H5(_))
    }

    /// The declared alternate-function pad map. See [`SpiPadMap`].
    pub(crate) fn pad_map(&self) -> SpiPadMap {
        self.pad_map
    }

    /// Set the declared AF pad map from the chip yaml.
    pub fn set_pad_map(&mut self, pad_map: SpiPadMap) {
        self.pad_map = pad_map;
    }

    /// `true` for the FIFO (G0/L4/F7/G4/WB) flavour of the STM32 layout.
    pub(crate) fn is_fifo_layout(&self) -> bool {
        matches!(&self.regs, SpiRegs::Stm32(r) if r.fifo)
    }

    /// Get-or-create the shared line-level cell (bus wiring hands the same
    /// `Arc` to the STM32 GPIO ports carrying this SPI's AF pads).
    /// Install the `GPIO_USARTROUTE` gate for this instance.
    pub(crate) fn set_route_gate(
        &mut self,
        gate: crate::peripherals::efr32::usart_route::RouteGate,
    ) {
        self.route_gate = Some(gate);
    }

    /// Can this block's traffic reach a pin at all?
    ///
    /// True when no route block is wired (every non-EFR32 family), so nothing
    /// else changes behaviour.
    fn efr32_reaches_a_pad(&self) -> bool {
        self.route_gate
            .as_ref()
            .is_none_or(|g| g.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub(crate) fn line_levels_arc(&mut self) -> Arc<SpiLineLevels> {
        if self.lines.is_none() {
            // SCK idles at the programmed polarity, and every layout spells
            // that differently: STM32 CR1.CPOL (bit 1), nRF52 CONFIG.CPOL
            // (bit 2, PS v1.11 §6.25.6.22 p737). Reading only the STM32 bit
            // here would park an nRF52 SPIM's clock at the wrong rest level and
            // make every mode-2/3 trace start with a phantom edge.
            let cpol = match &self.regs {
                SpiRegs::Stm32(r) => r.cr1 & (1 << 1) != 0,
                SpiRegs::Nrf52(r) => r.config & NRF52_CONFIG_CPOL != 0,
                SpiRegs::Efr32s2Usart(r) => r.ctrl & EFR_USART_CTRL_CLKPOL != 0,
                _ => false,
            };
            let levels = if matches!(self.regs, SpiRegs::Nrf52(_)) {
                SpiLineLevels::new_nrf(cpol)
            } else {
                SpiLineLevels::new(cpol)
            };
            // On a world net the STM32 engine states its drive per line
            // (`spi_net`); until it has, it drives nothing.
            if matches!(self.regs, SpiRegs::Stm32(_)) {
                for line in 0..SPI_LINES.len() {
                    levels
                        .pad_lines()
                        .set_net_idle(line, crate::peripherals::pad_lines::LineDrive::Input);
                }
            }
            self.lines = Some(Arc::new(levels));
        }
        self.lines.as_ref().unwrap().clone()
    }

    // ── FIFO-type STM32 SPI (G0/L4/G4/WB/F7) ────────────────────────────────
    //
    // RM0444 §35.5.9: two 32-bit FIFOs. A frame of up to 8 bits occupies one
    // byte of a FIFO, a wider one two. TXE is set while the TX FIFO is at or
    // below half full; RXNE while the RX FIFO holds at least the CR2.FRXTH
    // threshold (one byte with FRXTH=1, two with FRXTH=0); FTLVL/FRLVL report
    // both levels in SR; a frame arriving into a full RX FIFO is lost and sets
    // OVR. DR reads pop the RX FIFO by access size: an 8-bit read one byte, a
    // 16-bit read two (two packed frames at DS≤8, one frame above).

    /// Bytes one frame of the current CR2.DS occupies in a FIFO.
    fn fifo_frame_bytes(&self) -> u8 {
        match &self.regs {
            SpiRegs::Stm32(r) if (r.cr2 >> 8) & 0xF >= 0b1000 => 2,
            _ => 1,
        }
    }

    /// FTLVL in bytes: the frames waiting in the TX FIFO.
    fn fifo_tx_bytes(&self) -> u8 {
        let frames = u8::try_from(self.tx_queue.len()).unwrap_or(u8::MAX);
        frames
            .saturating_mul(self.fifo_frame_bytes())
            .min(FIFO_BYTES)
    }

    /// Room in the TX FIFO for one more frame of the current size.
    fn fifo_tx_has_room(&self) -> bool {
        self.fifo_tx_bytes() + self.fifo_frame_bytes() <= FIFO_BYTES
    }

    /// TXE on a FIFO port: the TX FIFO is at or below half full.
    fn fifo_txe(&self) -> bool {
        self.fifo_tx_bytes() <= FIFO_BYTES / 2
    }

    /// TXE as firmware reads it, on either STM32 flavour.
    pub(super) fn stm32_txe(&self) -> bool {
        match &self.regs {
            SpiRegs::Stm32(r) if r.fifo => self.fifo_txe(),
            SpiRegs::Stm32(r) => r.sr & 0x0002 != 0,
            _ => false,
        }
    }

    /// Re-derive RXNE from the RX FIFO level and CR2.FRXTH.
    fn fifo_sync_rxne(&self) {
        let frxth = matches!(&self.regs, SpiRegs::Stm32(r) if r.cr2 & FIFO_CR2_FRXTH != 0);
        let threshold = if frxth { 1 } else { 2 };
        self.stm32_rxne.set(self.fifo_rx.get().len >= threshold);
    }

    /// A received frame enters the RX FIFO. `false` when the FIFO had no room:
    /// the frame is lost and OVR is set, as on silicon.
    pub(super) fn fifo_rx_push(&mut self, word: u16) -> bool {
        let bytes = self.fifo_frame_bytes();
        let mut fifo = self.fifo_rx.get();
        let stored = fifo.push_frame(word, bytes);
        self.fifo_rx.set(fifo);
        if stored {
            if let SpiRegs::Stm32(r) = &mut self.regs {
                r.dr = word;
            }
        } else {
            self.fifo_ovr.set(true);
            self.fifo_ovr_dr_read.set(false);
        }
        self.fifo_sync_rxne();
        stored
    }

    /// Pop `bytes` (1 or 2) from the RX FIFO for a DR read of that size,
    /// little-endian. An empty FIFO reads 0.
    fn fifo_rx_pop(&self, bytes: u8) -> u16 {
        let mut fifo = self.fifo_rx.get();
        let mut v = 0u16;
        for i in 0..bytes {
            match fifo.pop() {
                Some(b) => v |= u16::from(b) << (8 * i),
                None => break,
            }
        }
        self.fifo_rx.set(fifo);
        self.fifo_sync_rxne();
        if self.fifo_ovr.get() {
            self.fifo_ovr_dr_read.set(true);
        }
        v
    }

    /// SR as firmware reads it on a FIFO port: TXE, RXNE, FTLVL and FRLVL are
    /// live functions of the two FIFOs.
    fn fifo_sr(&self, sr: u16) -> u16 {
        let mut v = sr & !(FIFO_SR_LEVELS | 0x0043);
        if self.fifo_ovr.get() {
            v |= 0x0040;
        }
        if self.stm32_rxne.get() {
            v |= 0x0001;
        }
        if self.fifo_txe() {
            v |= 0x0002;
        }
        v |= fifo_level_code(self.fifo_rx.get().len) << FIFO_SR_FRLVL_SHIFT;
        v |= fifo_level_code(self.fifo_tx_bytes()) << FIFO_SR_FTLVL_SHIFT;
        v
    }

    /// FIFO-type master in NSS pulse mode: SSOE=1 with hardware NSS (SSM=0),
    /// NSSP=1 and CPHA=0 (NSSP "has no meaning" at CPHA=1, RM0444 §35.7.2).
    pub(super) fn nssp_active(&self) -> bool {
        match &self.regs {
            SpiRegs::Stm32(r) if r.fifo => {
                r.cr1 & (1 << 2) != 0
                    && r.cr1 & (1 << 9) == 0
                    && r.cr1 & 1 == 0
                    && r.cr2 & (1 << 2) != 0
                    && r.cr2 & FIFO_CR2_NSSP != 0
            }
            _ => false,
        }
    }

    /// Derive the wire timing from the live registers. Reset values (the
    /// datasheet defaults: BR=0 → f_PCLK/2, CPOL=CPHA=0, MSB first, 8-bit
    /// frames — CR2 reset 0x0700 on FIFO ports, CR1.DFF=0 on classic ports)
    /// apply when firmware never programs them.
    fn stm32_frame_timing(&self) -> FrameTiming {
        let (cr1, cr2, fifo) = match &self.regs {
            SpiRegs::Stm32(r) => (r.cr1, r.cr2, r.fifo),
            _ => (0, 0, false),
        };
        let br = (cr1 >> 3) & 0x7;
        let bits = if fifo {
            // CR2.DS[3:0]: frame = DS+1 bits. Values below 0b0011 are reserved
            // and forced to 0b0111 (8-bit) at write time; reset is 0x0700.
            (((cr2 >> 8) & 0xF) as u8) + 1
        } else if cr1 & (1 << 11) != 0 {
            16 // CR1.DFF
        } else {
            8
        };
        FrameTiming {
            half_ticks: 1u32 << br,
            bits,
            cpol: cr1 & (1 << 1) != 0,
            cpha: cr1 & 1 != 0,
            lsb_first: cr1 & (1 << 7) != 0,
        }
    }

    /// The (SCK, MOSI, MISO) levels the wire carries in the current frame
    /// state. SCK: idle = CPOL; the bit period's active half is the second
    /// half at CPHA=0 (leading edge = sample) and the first half at CPHA=1
    /// (leading edge = shift, trailing = sample). Data lines hold the bit
    /// value for the whole bit period.
    fn stm32_frame_levels(f: &ActiveFrame) -> (bool, bool, bool) {
        let active_half = f.second_half != f.t.cpha;
        let sck = if active_half { !f.t.cpol } else { f.t.cpol };
        let bit = |v: u16| {
            if f.t.lsb_first {
                (v >> f.bit_idx) & 1 != 0
            } else {
                (v >> (f.t.bits - 1 - f.bit_idx)) & 1 != 0
            }
        };
        // Default: MISO holds the slave's answer bit for the whole bit period,
        // in the master's phase. An edge-sampled slave drives MISO on ITS OWN
        // shift edges, which for a mode mismatch lands half a bit period away
        // from the master's — so the wire (and the logic analyzer sampling it)
        // shows the real offset instead of a clean byte.
        let miso = match f.miso_halves {
            Some(halves) => {
                let h = 2 * u32::from(f.bit_idx) + u32::from(f.second_half);
                (halves >> h) & 1 != 0
            }
            None => bit(f.miso),
        };
        (sck, bit(f.mosi), miso)
    }

    /// Publish the current frame-state levels into the shared line cell (the
    /// cell pushes any transition into the logic tap at this exact moment).
    fn stm32_drive_levels(&self) {
        let (Some(f), Some(lines)) = (&self.frame, &self.lines) else {
            return;
        };
        if f.nss_pulse {
            // Between frames: SCK rests at CPOL, the data lines hold.
            lines.set(f.t.cpol, lines.mosi(), lines.miso());
            return;
        }
        let (sck, mosi, miso) = Self::stm32_frame_levels(f);
        lines.set(sck, mosi, miso);
    }

    /// Dequeue the next pending frame onto the wire. Consults the byte-level
    /// devices at this frame boundary: the broadcast answer (last non-zero
    /// response, same routing rule as always) is the byte MISO clocks out
    /// during this frame.
    fn stm32_start_next_frame(&mut self) {
        let Some(value) = self.tx_queue.pop_front() else {
            return;
        };
        let t = self.stm32_frame_timing();
        let mask = if t.bits >= 16 {
            0xFFFF
        } else {
            (1u16 << t.bits) - 1
        };
        let mosi = value & mask;
        let (miso, miso_halves) = match self.edge_slave {
            // ── Opt-in: edge-accurate slave ──────────────────────────────────
            // The MOSI waveform this frame will put on the wire is already
            // fully determined (value + timing), so the bits the slave latches
            // are simulated edge by edge HERE, before the device is consulted —
            // which is what keeps the device contract identical: still exactly
            // one `transfer()` per frame, at the same frame boundary, whose
            // answer still rides MISO during this same frame. The only
            // difference is that a mode-mismatched slave is handed the bits it
            // really sampled, and its answer is clocked back at its own edges.
            Some((s_cpol, s_cpha)) if !self.attached_devices.is_empty() => {
                let (prev_mosi, prev_miso) = self.edge_hold;
                let slave_rx = edge_slave_capture(&t.wire(), mosi, prev_mosi, s_cpol, s_cpha);
                let mut resp = 0u8;
                for dev in &mut self.attached_devices {
                    // Last device wins; 0x00 is a valid MISO byte (MAX31855 LSB).
                    resp = dev.transfer((slave_rx & 0xFF) as u8);
                }
                let (halves, master_rx) =
                    edge_miso_wire(&t.wire(), u16::from(resp), prev_miso, s_cpol, s_cpha);
                self.edge_hold = (
                    frame_bit(&t.wire(), mosi, t.bits - 1),
                    (halves >> (2 * u32::from(t.bits) - 1)) & 1 != 0,
                );
                (master_rx, Some(halves))
            }
            // ── Default: byte-level ──────────────────────────────────────────
            _ => {
                let miso = if !self.attached_devices.is_empty() {
                    let mosi_byte = (mosi & 0xFF) as u8;
                    // Last device wins; 0x00 is a valid MISO byte (MAX31855 LSB).
                    let mut miso_byte = 0u8;
                    for dev in &mut self.attached_devices {
                        miso_byte = dev.transfer(mosi_byte);
                    }
                    miso_byte as u16
                } else if self.loopback {
                    mosi
                } else {
                    0
                };
                (miso, None)
            }
        };
        self.net_rx = 0;
        self.frame = Some(ActiveFrame {
            t,
            mosi,
            miso,
            miso_halves,
            bit_idx: 0,
            second_half: false,
            ticks_left: t.half_ticks,
            nss_pulse: false,
        });
        if self.nssp_active() {
            // NSS pulse mode: NSS goes (back) low as the frame starts.
            self.net_master_nss(false);
        }
        self.stm32_drive_levels();
    }

    /// Advance the wire by `units` peripheral-clock cycles. Returns `true`
    /// when a completed frame wants the TXE interrupt raised (CR2.TXEIE).
    fn stm32_advance_units(&mut self, units: u64) -> bool {
        self.stm32_advance_units_from(None, units)
    }

    /// As [`Self::stm32_advance_units`], with `base` the engine cycle the
    /// advance starts from when it is known: each boundary's wire edges are
    /// then stamped at the boundary's own cycle, not when the event was
    /// drained.
    fn stm32_advance_units_from(&mut self, base: Option<u64>, mut units: u64) -> bool {
        let mut irq = false;
        let mut t = base;
        while units > 0 {
            let Some(f) = &mut self.frame else { break };
            let step = (f.ticks_left as u64).min(units);
            f.ticks_left -= step as u32;
            units -= step;
            if f.ticks_left == 0 {
                if let Some(t) = t.as_mut() {
                    *t += step;
                }
                self.stm32_set_stamp(t);
                self.stm32_segment_boundary(&mut irq);
            } else if let Some(t) = t.as_mut() {
                *t += step;
            }
        }
        self.stm32_set_stamp(None);
        irq
    }

    /// Stamp this controller's wire reports (see `PadLines::set_stamp`).
    fn stm32_set_stamp(&self, cycle: Option<u64>) {
        if let Some(lines) = &self.lines {
            lines.pad_lines().set_stamp(cycle);
        }
    }

    /// The current half-period expired: drive the next segment, or complete
    /// the frame at derived wire time (RXNE/BSY/TXE flip HERE, not at the DR
    /// write).
    fn stm32_segment_boundary(&mut self, irq: &mut bool) {
        let Some(mut f) = self.frame.take() else {
            return;
        };
        if f.nss_pulse {
            // The NSS-high pulse between two frames is over: the queued frame
            // starts.
            self.stm32_start_next_frame();
            if self.frame.is_none() {
                self.stm32_idle(f.t.cpol, irq);
            }
            return;
        }
        if !f.second_half {
            f.second_half = true;
            f.ticks_left = f.t.half_ticks;
            self.frame = Some(f);
            self.stm32_drive_levels();
            // Into the second half is the sampling edge in both phases.
            if self.net_mode() {
                self.net_master_sample();
            }
            return;
        }
        if f.bit_idx + 1 < f.t.bits {
            f.bit_idx += 1;
            f.second_half = false;
            f.ticks_left = f.t.half_ticks;
            self.frame = Some(f);
            self.stm32_drive_levels();
            return;
        }
        // Frame complete: the exchange lands in the RX path.
        //
        // In full-duplex master mode silicon ALWAYS completes the receive: the
        // shift register samples the MISO line every frame and RXNE asserts
        // when the RX buffer fills, whether or not a slave is driving. With no
        // slave the captured value is just the idle line level — it is not an
        // absent event. Gating RXNE on an attached device made every polling
        // driver hang forever waiting for a flag that could never arrive
        // (`SPI.transfer()` on an unpopulated bus, which is the common case in
        // a simulator); found by the Arduino conformance sketch on F401.
        let rx_fifo = self.rx_fifo;
        // On a net, MISO is whatever the wire carried at each sampling edge:
        // a peer slave's answer, a pull, or nothing at all.
        let net_rx = (self.net_mode() && self.attached_devices.is_empty() && !self.loopback)
            .then_some(self.net_rx);
        let driven = self.loopback || !self.attached_devices.is_empty() || net_rx.is_some();
        let word = net_rx.unwrap_or(f.miso);
        if !rx_fifo {
            if let SpiRegs::Stm32(r) = &mut self.regs {
                r.dr = word;
                // Classic F1/F4 port: no FIFO, RXNE on every frame.
                r.sr |= 0x0001;
                self.stm32_rxne.set(true);
            }
            if net_rx.is_some() && self.net_master_rx_irq() {
                *irq = true;
            }
        } else {
            // FIFO port: the frame enters the RX FIFO and RXNE follows the
            // CR2.FRXTH threshold. FRXTH=1 → 8 bit, so one 8-bit frame
            // asserts it; FRXTH=0 (reset) → 16 bit, so a single 8-bit frame
            // must NOT. This is what a real NUCLEO-L476RG reports (SR=0x0002
            // after TX), and is why STM32duino sets FRXTH before 8-bit
            // transfers.
            // A slave (or loopback) must actually drive MISO for the FIFO to
            // fill: a real NUCLEO-L476RG with nothing wired reports SR=0x0002
            // after a transmit even with FRXTH=1, which is the value pinned by
            // test_nucleo_l476rg_spi_survival.
            if let SpiRegs::Stm32(r) = &mut self.regs {
                r.dr = word;
            }
            if driven {
                self.fifo_rx_push(word);
                if net_rx.is_some() && self.net_master_rx_irq() && self.stm32_rxne.get() {
                    *irq = true;
                }
            }
        }
        if !self.tx_queue.is_empty() {
            if self.nssp_active() {
                // NSS pulse mode: NSS rises for one SCK period before the
                // next frame (RM0444 §35.5.5). BSY stays set across the pulse.
                self.net_master_nss(true);
                self.frame = Some(ActiveFrame {
                    nss_pulse: true,
                    ticks_left: 2 * f.t.half_ticks,
                    ..f
                });
                self.stm32_drive_levels();
                return;
            }
            // Back-to-back: the next queued frame starts on the very next
            // cycle (BSY stays set, TXE stays clear).
            self.stm32_start_next_frame();
            return;
        }
        self.stm32_idle(f.t.cpol, irq);
    }

    /// The last frame is out and nothing is queued: BSY drops, TXE rises,
    /// the wire idles.
    fn stm32_idle(&mut self, cpol: bool, irq: &mut bool) {
        self.frame = None;
        if let SpiRegs::Stm32(r) = &mut self.regs {
            r.sr &= !0x0080; // Clear BSY
            r.sr |= 0x0002; // Set TXE
            if (r.cr2 & (1 << 7)) != 0 {
                *irq = true; // TXEIE
            }
        }
        if self.nssp_active() {
            // NSS pulse mode, single transfer: NSS is forced high after it.
            self.net_master_nss(true);
        }
        // Wire idles: SCK returns to CPOL (the trailing edge of the last
        // bit); MOSI/MISO hold their last driven level, like real pads.
        if let Some(lines) = &self.lines {
            lines.set(cpol, lines.mosi(), lines.miso());
        }
    }

    /// Peripheral-clock cycles until the engine's next wire transition (the
    /// scheduling quantum for the event-driven path). 0 when idle.
    fn stm32_next_transition_ticks(&self) -> u64 {
        self.frame
            .as_ref()
            .map(|f| f.ticks_left.max(1) as u64)
            .unwrap_or(0)
    }

    // ── STM32 DMA requests (CR2.TXDMAEN / RXDMAEN, LDMA_TX / LDMA_RX) ───────

    /// The DMA request lines that are high and not yet raised: `(tx, rx)`.
    ///
    /// * **TX** (RM0444 §35.5.10, RM0090 §28.3.9): "a DMA request is issued
    ///   each time TXE is set to 1" — the classic port's one-word buffer is
    ///   empty, the FIFO port's TX FIFO is at or below half full. Raised only
    ///   while SPE=1: the model does not latch a DR write into a disabled
    ///   master, and every driver (HAL included) enables SPE before TXDMAEN.
    /// * **RX**: "each time RXNE is set to 1" — on the FIFO port RXNE follows
    ///   CR2.FRXTH. With LDMA_RX=1 and FRXTH=0, once the DMA has said the next
    ///   item is its last (see [`crate::Peripheral::dma_access_notice`]), the
    ///   request is raised for a single byte: the odd frame of a packed
    ///   transfer.
    fn stm32_dma_due(&self) -> (bool, bool) {
        match &self.regs {
            SpiRegs::Stm32(r) => {
                if r.cr2 & (CR2_TXDMAEN | CR2_RXDMAEN) == 0 {
                    return (false, false);
                }
                let tx = r.cr2 & CR2_TXDMAEN != 0
                    && !self.dma_tx_req
                    && r.cr1 & (1 << 6) != 0
                    && self.stm32_txe();
                let rx_level = self.stm32_rxne.get()
                    || (r.fifo
                        && r.cr2 & FIFO_CR2_LDMA_RX != 0
                        && r.cr2 & FIFO_CR2_FRXTH == 0
                        && self.dma_rx_last
                        && self.fifo_rx.get().len >= 1);
                let rx = r.cr2 & CR2_RXDMAEN != 0 && !self.dma_rx_req.get() && rx_level;
                (tx, rx)
            }
            // SPI v3 (H5/U5): DMA enables are CFG1.RXDMAEN/TXDMAEN, not CR2
            // (RM0456 §68.8.3). TX is due while the master is started and the
            // TX FIFO has space; RX is due while a frame is waiting in RXDR.
            SpiRegs::Stm32H5(r) => {
                let tx = r.cfg1 & H5_CFG1_TXDMAEN != 0
                    && !self.dma_tx_req
                    && r.cr1 & H5_CR1_SPE != 0
                    && r.cr1 & H5_CR1_CSTART != 0
                    && r.sr & H5_SR_TXP != 0;
                let rx = r.cfg1 & H5_CFG1_RXDMAEN != 0
                    && !self.dma_rx_req.get()
                    && !self.h5_rx_fifo.borrow().is_empty();
                (tx, rx)
            }
            _ => (false, false),
        }
    }

    /// The H5 "SPI v3" interrupt line is the level `SR & IER` over the
    /// error/event flags (RM0456 §68.8.5/68.8.6). It has no frame engine to
    /// raise it, so the bus asks this before arming a wakeup.
    fn h5_irq_pending(&self) -> bool {
        match &self.regs {
            SpiRegs::Stm32H5(r) => (self.h5_live_sr(r) & r.ier & 0x7FF) != 0,
            _ => false,
        }
    }

    /// SR with the receive-queue-dependent flags applied: RXP/DXP are live
    /// over the queue, everything else is the stored event register.
    fn h5_rx_available(&self) -> bool {
        !self.h5_rx_fifo.borrow().is_empty() || self.h5_rx_cur.get().is_some()
    }

    fn h5_live_sr(&self, r: &Stm32H5SpiRegs) -> u32 {
        let mut v = r.sr;
        if self.h5_rx_available() {
            v |= H5_SR_RXP;
            if v & H5_SR_TXP != 0 {
                v |= H5_SR_DXP;
            }
        } else {
            v &= !(H5_SR_RXP | H5_SR_DXP);
        }
        v
    }

    /// Raise every due DMA request: the `dma_signals` the bus routes to the
    /// DMA channel(s) the chip's routing registers select. Each request stays
    /// outstanding until the DR access that answers it.
    pub(crate) fn stm32_take_dma_signals(&mut self) -> Vec<u32> {
        let (tx, rx) = self.stm32_dma_due();
        let mut out = Vec::new();
        if rx {
            self.dma_rx_req.set(true);
            out.push(crate::DMA_SIGNAL_RX);
        }
        if tx {
            self.dma_tx_req = true;
            out.push(crate::DMA_SIGNAL_TX);
        }
        out
    }

    /// The `SPI_DONE_TOKEN` event with no frame on the wire: raise the due DMA
    /// requests and keep re-checking (on this same token) while any were
    /// raised. `scheduled` stays set while the re-check is in flight, so a
    /// frame started meanwhile is caught by that event's early-wakeup path.
    fn dma_recheck_result(&mut self) -> crate::sched::EventResult {
        let dma_signals = self.stm32_take_dma_signals();
        let again = !dma_signals.is_empty();
        self.scheduled = again;
        crate::sched::EventResult {
            dma_signals,
            reschedule_delay: again.then_some(SPI_DMA_RECHECK),
            ..Default::default()
        }
    }

    /// `true` when a DMA request is due (one branch while no DMA is enabled).
    fn stm32_dma_pending(&self) -> bool {
        let (tx, rx) = self.stm32_dma_due();
        tx || rx
    }
}

impl crate::Peripheral for Spi {
    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn crate::peripherals::device::SpiDevice>>> {
        Some(&self.attached_devices)
    }

    fn spi_attached_devices_mut(
        &mut self,
    ) -> Option<&mut Vec<Box<dyn crate::peripherals::device::SpiDevice>>> {
        Some(&mut self.attached_devices)
    }

    /// This controller hosts off-chip SPI devices, so the machine's central
    /// device-time drive fans elapsed µs out to them — the same drive, the same
    /// deltas and the same source the attached I²C slaves get. Phase A of the
    /// YAML device machine: a declarative device must not be able to tell which
    /// bus it hangs off by how much time it is told has passed.
    fn drives_central_device_time(&self) -> bool {
        true
    }

    fn advance_attached_device_time_us(&mut self, us: u64) {
        if us == 0 {
            return;
        }
        for dev in &mut self.attached_devices {
            dev.advance_time_us(us);
        }
    }

    /// Tier 2: collect this controller's attached devices' pin drives.
    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for dev in &mut self.attached_devices {
            crate::peripherals::device::drain_spi_pin_drives(&mut **dev, out);
        }
    }

    /// EVERY layout here publishes SCK/MOSI/MISO now.
    ///
    /// The EFR32 USART-as-SPI path was the last exception, and it did not
    /// publish until 2026-08-22: a frame completed inside the `TXDATA` write
    /// and no edge existed, so naming the lines would have been a pad-table row
    /// promising a trace that stayed flat — which is what `bus_visibility`
    /// exists to catch. `efr32_wire_flush` is what earns the names, and the
    /// match this used to need went with it.
    fn line_names(&self) -> &'static [&'static str] {
        // NSS is a wire only on a world `gpio_net` (see `spi_net`); off a
        // net nothing drives it, and naming it would promise edges never made.
        if self.net_mode() {
            SPI_LINES
        } else {
            &SPI_LINES[..3]
        }
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_ref().map(|levels| &**levels.pad_lines())
    }

    /// The nRF52 SPIM names its pads in `PSEL` (see `wire_nrf52_pads`).
    fn claims_pads(&self) -> bool {
        self.is_nrf()
    }

    fn wire_input_edge(&mut self, line: usize, level: bool, cycle: u64) -> bool {
        self.stm32_set_stamp(Some(cycle));
        let wake = self.net_slave_edge(line, level);
        self.stm32_set_stamp(None);
        // A frame the edge completed (or a transmit word it took) may raise a
        // DMA request: the bus then harvests the request event.
        wake || (!self.dma_scheduled && self.stm32_dma_pending())
    }

    fn dma_access_notice(&mut self, offset: u64, read: bool, remaining: u32) {
        if offset & !3 != 0x0C || !matches!(self.regs, SpiRegs::Stm32(_)) {
            return;
        }
        if read {
            // `remaining == 1`: the read after this one ends the transfer.
            self.dma_rx_last = remaining == 1;
        } else {
            // This write ends the transfer.
            self.dma_tx_last = remaining == 0;
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = match &self.regs {
            SpiRegs::Nrf52(r) => r.read_reg(reg_offset),
            SpiRegs::KinetisDspi(r) => r.read_reg(reg_offset),
            SpiRegs::Stm32H5(r) => {
                let mut v = r.read_reg(reg_offset) as u32;
                // RXDR read consumes the frame: RXP clears and an outstanding
                // RX DMA request is answered (RM0456 §68.8.6).
                if reg_offset == 0x30 {
                    // Serve one byte lane of the current frame; a fresh frame
                    // is popped on the first lane read (the bus is byte-wide).
                    let cur = match self.h5_rx_cur.get() {
                        Some(cur) => cur,
                        None => {
                            let next = self.h5_rx_fifo.borrow_mut().pop_front();
                            match next {
                                Some(frame) => {
                                    self.h5_rx_cur.set(Some(frame));
                                    frame
                                }
                                None => (0, 1),
                            }
                        }
                    };
                    let (word, bytes) = cur;
                    if (byte_offset as u8) < bytes {
                        // Keep the whole frame: the tail of `read` extracts
                        // this access's byte lane from `reg_val` again.
                        v = word;
                    }
                    if (byte_offset as u8) + 1 >= bytes {
                        self.h5_rx_cur.set(None);
                        self.dma_rx_req.set(false);
                    }
                    self.stm32_rxne.set(self.h5_rx_available());
                }
                // SR.RXP/DXP are live over the queue, not the stored bits.
                if reg_offset == 0x14 {
                    v = self.h5_live_sr(r);
                }
                v
            }
            SpiRegs::Efr32s2Usart(r) => r.read_reg(reg_offset),
            // Widen u16→u32 before the shift: byte accesses at offsets 2/3 read
            // the upper byte of the next halfword; `(u16 as u32) >> 16` is 0
            // without an overflow panic under the CI release profile.
            SpiRegs::Stm32(r) if r.fifo => match reg_offset {
                // DR, 8-bit access: pops one byte of the RX FIFO. The upper
                // byte lane alone pops nothing.
                0x0C if byte_offset == 0 => {
                    self.dma_rx_req.set(false);
                    return Ok(self.fifo_rx_pop(1) as u8);
                }
                0x0C => return Ok(0),
                0x08 => {
                    let v = self.fifo_sr(r.sr);
                    if byte_offset == 0 && self.fifo_ovr_dr_read.get() {
                        // DR then SR: OVR clears (after this read shows it).
                        self.fifo_ovr.set(false);
                        self.fifo_ovr_dr_read.set(false);
                    }
                    u32::from(v)
                }
                _ => u32::from(r.read_reg(reg_offset)),
            },
            SpiRegs::Stm32(r) => {
                // DR read: clear-on-read RXNE (silicon). Must happen on every
                // access size that drains RX, including the byte path HAL uses.
                if reg_offset == 0x0C {
                    self.stm32_rxne.set(false);
                    self.dma_rx_req.set(false);
                }
                let mut v = r.read_reg(reg_offset) as u32;
                if reg_offset == 0x08 {
                    // SR: expose live RXNE from the Cell (authoritative after DR).
                    if self.stm32_rxne.get() {
                        v |= 0x0001;
                    } else {
                        v &= !0x0001;
                    }
                }
                v
            }
        };
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        if let SpiRegs::Nrf52(_) = &self.regs {
            // Legacy SPI (Arduino SPI.cpp): write TXD @ 0x51C with ENABLE=1
            // clocks one byte immediately and raises EVENTS_READY.
            if reg_offset == 0x51C && byte_offset == 0 {
                let enable = match &self.regs {
                    SpiRegs::Nrf52(r) => r.enable & 0xF,
                    _ => 0,
                };
                if enable == NRF52_ENABLE_SPI {
                    self.nrf52_legacy_spi_txd(value);
                    return Ok(());
                }
            }
            let cur = match &self.regs {
                SpiRegs::Nrf52(r) => r.read_reg(reg_offset),
                _ => 0,
            };
            let mask: u32 = 0xFF << (byte_offset * 8);
            let new = (cur & !mask) | ((value as u32) << (byte_offset * 8));
            let start_triggered = if let SpiRegs::Nrf52(r) = &mut self.regs {
                r.write_reg(reg_offset, new)
            } else {
                false
            };
            // ENABLE (0x500) and PSEL.SCK/MOSI (0x508/0x50C) all move which pad
            // this controller's wire reaches. Republishing on every nRF write
            // rather than only on those three offsets keeps the ONE call site:
            // `update` is two comparisons when nothing moved.
            self.sync_nrf_pin_claims();
            self.nrf_after_write(reg_offset, new, start_triggered);
            return Ok(());
        }

        // STM32H5: 32-bit registers — read-modify-write merge the byte, then
        // hand the full word to the register handler. The write-only registers
        // (TXDR, IFCR) read back 0, so the merge degenerates to the bare byte
        // shifted into place — a byte write to TXDR is one 8-bit frame, which
        // matches RM0481 §41.4.13 (TXDR access size = frame size).
        if let SpiRegs::Stm32H5(_) = &self.regs {
            let cur = match &self.regs {
                SpiRegs::Stm32H5(r) => r.read_reg(reg_offset),
                _ => 0,
            };
            let mask: u32 = 0xFF << (byte_offset * 8);
            let new = (cur & !mask) | ((value as u32) << (byte_offset * 8));
            self.write_stm32h5_reg(reg_offset, new);
            return Ok(());
        }

        // EFR32 Series-2 USART: 32-bit registers. Read-modify-write the byte
        // then hand the full word over, so a byte write of TXDATA still clocks
        // exactly one frame.
        if let SpiRegs::Efr32s2Usart(_) = &self.regs {
            let cur = match &self.regs {
                SpiRegs::Efr32s2Usart(r) => r.read_reg(reg_offset),
                _ => 0,
            };
            let mask: u32 = 0xFF << (byte_offset * 8);
            let new = (cur & !mask) | ((value as u32) << (byte_offset * 8));
            self.write_efr32s2_usart_reg(reg_offset, new);
            return Ok(());
        }

        // Kinetis DSPI: 32-bit registers, read-modify-write the byte then hand
        // the full word to the register handler (PUSHR reads back 0, so a byte
        // write degenerates to the shifted byte).
        if let SpiRegs::KinetisDspi(_) = &self.regs {
            let cur = match &self.regs {
                SpiRegs::KinetisDspi(r) => r.read_reg(reg_offset),
                _ => 0,
            };
            let mask: u32 = 0xFF << (byte_offset * 8);
            let new = (cur & !mask) | ((value as u32) << (byte_offset * 8));
            self.write_kinetis_dspi_reg(reg_offset, new);
            return Ok(());
        }

        // STM32: same widen-then-shift dance to avoid u16 shift overflow; the
        // final write truncates back to u16, discarding bytes 2..3.
        let cur = match &self.regs {
            SpiRegs::Stm32(r) => r.read_reg(reg_offset) as u32,
            _ => 0,
        };
        let mask: u32 = 0xFF << (byte_offset * 8);
        let new = (cur & !mask) | ((value as u32) << (byte_offset * 8));
        self.write_stm32_reg(reg_offset, new as u16);
        Ok(())
    }

    /// For nRF52 SPIM, 32-bit register writes must be handled atomically so
    /// that INTENSET / INTENCLR (set/clear bitmask registers) receive the full
    /// 32-bit value rather than a read-modify-write merge of individual bytes.
    /// The byte-merge in the default `write_u32` would incorrectly OR in bits
    /// from the current register state and cause INTENCLR to clear more bits
    /// than intended. Firmware on Cortex-M always uses STR (32-bit) for
    /// nRF register accesses — this override matches that behaviour.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // H5 "SPI v3": a 32-bit register write is one transaction (TXDR's word
        // is one 32-bit frame, CFG/IER are stored atomically).
        if let SpiRegs::Stm32H5(_) = &self.regs {
            self.write_stm32h5_reg(offset & !3, value);
            return Ok(());
        }
        if let SpiRegs::Nrf52(_) = &self.regs {
            let reg_offset = offset & !3;
            // Legacy SPI TXD: same immediate-byte path as the u8 write arm.
            if reg_offset == 0x51C {
                let enable = match &self.regs {
                    SpiRegs::Nrf52(r) => r.enable & 0xF,
                    _ => 0,
                };
                if enable == NRF52_ENABLE_SPI {
                    self.nrf52_legacy_spi_txd(value as u8);
                    return Ok(());
                }
            }
            let start_triggered = if let SpiRegs::Nrf52(r) = &mut self.regs {
                r.write_reg(reg_offset, value)
            } else {
                false
            };
            // See the byte-write path: one call site for every pad-moving write.
            self.sync_nrf_pin_claims();
            self.nrf_after_write(reg_offset, value, start_triggered);
            return Ok(());
        }
        // STM32H5: 32-bit registers must be written atomically — a word write
        // to TXDR is ONE frame (byte-splitting would transmit four), and IFCR
        // (write-1-to-clear) must see the full mask in a single access.
        if let SpiRegs::Stm32H5(_) = &self.regs {
            self.write_stm32h5_reg(offset & !3, value);
            return Ok(());
        }
        // EFR32 Series-2 USART: a word write of TXDATA is ONE frame. Byte
        // splitting would clock four.
        if let SpiRegs::Efr32s2Usart(_) = &self.regs {
            self.write_efr32s2_usart_reg(offset & !3, value);
            return Ok(());
        }
        // Kinetis DSPI: PUSHR is one 32-bit frame push — must be atomic (byte
        // splitting would transmit spurious frames), so handle the word here.
        if let SpiRegs::KinetisDspi(_) = &self.regs {
            self.write_kinetis_dspi_reg(offset & !3, value);
            return Ok(());
        }
        // STM32 classic/FIFO DR: a word store is ONE bus access, so ONE
        // frame (bits 31:16 are not part of the register). Splitting it into
        // byte writes re-wrote DR four times and clocked four frames, which a
        // 16-bit (`DFF`) master on a net made visible.
        if matches!(self.regs, SpiRegs::Stm32(_)) && offset == 0x0C {
            return self.write_u16(0x0C, value as u16);
        }
        // STM32 default: four byte writes.
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write(offset + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write(offset + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    /// FIFO-type STM32 DR: a halfword read is ONE access popping two bytes
    /// of the RX FIFO (two packed frames at DS≤8, one frame above). The
    /// byte-wise default would pop the low byte and read 0 for the high one.
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        if offset == 0x0C && self.is_fifo_layout() {
            self.dma_rx_req.set(false);
            return Ok(self.fifo_rx_pop(2));
        }
        let b0 = self.read(offset)? as u16;
        let b1 = self.read(offset + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }

    /// As [`Self::read_u16`]: DR is a 16-bit register, a word read of it is
    /// one halfword-sized pop (bits 31:16 read 0).
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset == 0x0C && self.is_fifo_layout() {
            self.dma_rx_req.set(false);
            return Ok(u32::from(self.fifo_rx_pop(2)));
        }
        let b0 = self.read(offset)? as u32;
        let b1 = self.read(offset + 1)? as u32;
        let b2 = self.read(offset + 2)? as u32;
        let b3 = self.read(offset + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if self.is_nrf() {
            self.write(offset, (value & 0xFF) as u8)?;
            self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
            return Ok(());
        }
        // STM32H5: a halfword TXDR access is ONE 16-bit frame (RM0481
        // §41.4.13) — byte-splitting would transmit two frames. The classic
        // 0x0C special-case below must not run either: 0x0C is CFG2 on H5.
        if let SpiRegs::Stm32H5(_) = &self.regs {
            // H5 registers are 32-bit. A halfword access is ONE transaction:
            // TXDR's halfword is one 16-bit frame (RM0456 §68.8.13), and a
            // config register's upper half must merge into the stored word.
            let reg_offset = offset & !3;
            let byte_offset = (offset % 4) as u32;
            let cur = match &self.regs {
                SpiRegs::Stm32H5(r) => r.read_reg(reg_offset),
                _ => 0,
            };
            let mask: u32 = 0xFFFF << (byte_offset * 8);
            let new = (cur & !mask) | (u32::from(value) << (byte_offset * 8));
            self.write_stm32h5_reg(reg_offset, new);
            return Ok(());
        }
        // SPI DR (offset 0x0C) MUST be atomic — a Thumb `strh` is one bus
        // access kicking off a single transfer. Byte-splitting would start two
        // transfers and broadcast a spurious upper byte to attached devices.
        if offset == 0x0C {
            let (fifo, ds) = match &self.regs {
                SpiRegs::Stm32(r) => (r.fifo, (r.cr2 >> 8) & 0xF),
                _ => (false, 0),
            };
            let ldma_tx = matches!(&self.regs, SpiRegs::Stm32(r) if r.cr2 & FIFO_CR2_LDMA_TX != 0);
            if fifo && ds <= 0b0111 && ldma_tx && self.dma_tx_last {
                // LDMA_TX: the last DMA item of an odd-length packed transfer
                // carries one frame (RM0444 §35.5.10); its upper byte is not
                // data.
                self.write_stm32_reg(0x0C, value & 0xFF);
            } else if fifo && ds <= 0b0111 {
                // FIFO data packing (RM0351 §40.4.9): a 16-bit DR access at
                // DS≤8 enqueues TWO frames (low byte, then high byte).
                self.write_stm32_reg(0x0C, value & 0xFF);
                self.write_stm32_reg(0x0C, (value >> 8) & 0xFF);
            } else {
                self.write_stm32_reg(0x0C, value);
            }
            return Ok(());
        }
        // Other registers: byte-split is fine (no transfer side-effects).
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    /// Event-scheduler path: anchor the engine's wire state to the current
    /// CPU cycle. The bus calls this before every MMIO write, so a DR write
    /// pins the frame start to the batch-start cycle — and because CPU batches
    /// never cross a peripheral-tick boundary, that cycle is identical whether
    /// the run loop is clamped (poll capture) or batched (push capture).
    fn sync_to(&mut self, tick_now: u64) {
        if tick_now <= self.anchor_tick {
            return;
        }
        let delta = tick_now - self.anchor_tick;
        let base = self.anchor_tick;
        self.anchor_tick = tick_now;
        if self.frame.is_some() {
            self.stm32_advance_units_from(Some(base), delta);
        }
    }

    /// Event-driven clocking (the walk-deleted path): while a frame is on the
    /// wire, the engine keeps exactly one event armed at its next wire
    /// transition. The returned delay is relative to the just-synced anchor;
    /// the bus converts it to the absolute deadline `anchor + 1 + delay`, so
    /// the `- 1` here lands the event exactly at `anchor + half_ticks` — the
    /// first transition's true cycle at any tick interval. [`Self::on_event`]
    /// self-corrects against the absolute anchor (a drain may run past the
    /// deadline by up to one tick interval) and re-arms via `reschedule_delay`
    /// until the frame (and any queued frames) complete.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        // nRF52 SPIM on a world `gpio_net`: the bit engine's own chain.
        if self.nrf_net_mode() || self.nrf_net_needs_bus() {
            return self.nrf_net_take_events();
        }
        // nRF52 SPIM EasyDMA: delay-0 completion (next cycle after TASKS_START).
        if self.nrf52_pending_start {
            return vec![(0, SPI_NRF52_EASYDMA_TOKEN)];
        }
        let mut events = Vec::new();
        if self.net_irq_pending && !self.net_irq_scheduled {
            self.net_irq_scheduled = true;
            events.push((0, SPI_NET_IRQ_TOKEN));
        }
        if !self.dma_scheduled && self.stm32_dma_pending() {
            self.dma_scheduled = true;
            events.push((0, SPI_DMA_TOKEN));
        }
        if self.frame.is_some() && !self.scheduled {
            self.scheduled = true;
            events.push((
                self.stm32_next_transition_ticks().saturating_sub(1),
                SPI_DONE_TOKEN,
            ));
            return events;
        }
        if !events.is_empty() {
            return events;
        }
        if self.h5_irq_pending() {
            return vec![(0, SPI_H5_IRQ_TOKEN)];
        }
        // H5 "SPI v3": no bit engine, so no frame to chase — but a buffered
        // narration burst still needs a wakeup to publish on. Arm only while a
        // burst is genuinely held, which requires routed pads, and only once
        // per chain, so an N-frame burst costs ONE wakeup rather than N.
        if !self.h5_wire_words.is_empty() && !self.h5_scheduled {
            self.h5_arm_seq = self.h5_arm_seq.wrapping_add(1);
            self.h5_scheduled = true;
            return vec![(self.h5_wire_ready_in(), h5_wire_token(self.h5_arm_seq))];
        }
        // ⚠️ The EFR32 burst needs the SAME wakeup, and for a reason that only
        // shows up under `event-scheduler`: with the feature on, the legacy
        // walk skips this model, so the `tick_elapsed` retry never runs and a
        // burst written back-to-back stays held forever. Measured — the
        // `bus_visibility` ratchet decoded ONE byte of three (`[53]` of
        // `[53, 1c, e1]`) with the feature on and all three with it off, which
        // is a feature flag changing what a trace says.
        if !self.efr32_wire_bytes.is_empty() && !self.efr32_scheduled {
            self.efr32_scheduled = true;
            return vec![(self.efr32_wire_pending_cycles(), SPI_EFR32_WIRE_TOKEN)];
        }
        Vec::new()
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token == nrf_net::NRF_NET_TOKEN {
            return self.nrf_net_on_event(sched.now(), bus);
        }
        if event_token == SPI_NRF52_EASYDMA_TOKEN {
            if self.nrf52_pending_start {
                self.do_nrf52_easydma(bus);
            }
            return crate::sched::EventResult::default();
        }
        if event_token == SPI_NET_IRQ_TOKEN {
            self.net_irq_scheduled = false;
            let raise = std::mem::take(&mut self.net_irq_pending);
            return crate::sched::EventResult {
                raise_own_irq: raise,
                dma_signals: self.stm32_take_dma_signals(),
                ..Default::default()
            };
        }
        if event_token == SPI_DMA_TOKEN {
            // Raise the due requests, then look again shortly while they
            // are being answered: an answered request whose line is still
            // high (a FIFO with room for more, or still holding a frame) is
            // raised again. The chain ends once nothing new is due.
            let dma_signals = self.stm32_take_dma_signals();
            let again = !dma_signals.is_empty();
            self.dma_scheduled = again;
            return crate::sched::EventResult {
                dma_signals,
                reschedule_delay: again.then_some(SPI_DMA_RECHECK),
                ..Default::default()
            };
        }
        if event_token == SPI_EFR32_WIRE_TOKEN {
            self.efr32_wire_flush();
            let pending = !self.efr32_wire_bytes.is_empty();
            self.efr32_scheduled = pending;
            return crate::sched::EventResult {
                // Ask for the wire time the burst still needs, floored at one
                // cycle so a held burst always converges instead of spinning.
                reschedule_delay: pending.then(|| self.efr32_wire_pending_cycles().max(1)),
                ..Default::default()
            };
        }
        if event_token == SPI_H5_IRQ_TOKEN {
            let pending = self.h5_irq_pending();
            return crate::sched::EventResult {
                raise_own_irq: pending,
                reschedule_delay: pending.then_some(1),
                ..Default::default()
            };
        }
        if event_token & SPI_H5_WIRE_TOKEN_FLAG != 0 {
            if event_token != h5_wire_token(self.h5_arm_seq) {
                // Stale token from a superseded arm; the live chain owns
                // publication.
                return crate::sched::EventResult::default();
            }
            self.h5_wire_flush(self.h5_wire_words.len() >= H5_WIRE_BURST_CAP);
            // A flush that reported LevelsOnly keeps its words; `h5_wire_ready_in`
            // is then 0, so `max(1)` retries on the next cycle and converges as
            // the run grows. A successful flush leaves nothing and the chain
            // stops.
            let pending = !self.h5_wire_words.is_empty();
            self.h5_scheduled = pending;
            return crate::sched::EventResult {
                // `h5_wire_pending_cycles`, NOT `h5_wire_ready_in`: a burst
                // still held after a forced flush is one the cycle window had
                // no room for, and `h5_wire_ready_in` reports 0 at the cap —
                // which would retry every cycle, rebuilding the whole plan each
                // time. Ask for the wire time the burst actually still needs.
                reschedule_delay: pending.then(|| self.h5_wire_pending_cycles().max(1)),
                ..Default::default()
            };
        }

        self.scheduled = false;
        let Some(f) = &self.frame else {
            return self.dma_recheck_result();
        };
        let mut res = crate::sched::EventResult::default();
        let now = sched.now();
        let target = self.anchor_tick + f.ticks_left as u64;
        if now < target {
            // Early wakeup (a stale event from before a re-anchor): re-arm at
            // the exact boundary.
            res.reschedule_delay = Some(target - now);
            self.scheduled = true;
            return res;
        }
        // Advance the wire to "now" — at tick interval 1 drains run every
        // cycle, so this is exactly one boundary; at larger intervals a drain
        // may arrive up to one interval late and cross several boundaries in
        // one call, but the boundaries' derived cycles (and the frame's total
        // wire time) are unchanged.
        let delta = now - self.anchor_tick;
        let base = self.anchor_tick;
        self.anchor_tick = now;
        if self.stm32_advance_units_from(Some(base), delta) {
            res.raise_own_irq = true; // TXEIE at frame completion
        }
        if self.frame.is_some() {
            res.reschedule_delay = Some(self.stm32_next_transition_ticks());
            self.scheduled = true;
            res.dma_signals = self.stm32_take_dma_signals();
        } else {
            // The last frame is out: its RXNE / TXE may raise DMA requests,
            // re-checked like the `SPI_DMA_TOKEN` chain.
            let dma = self.dma_recheck_result();
            res.dma_signals = dma.dma_signals;
            res.reschedule_delay = dma.reschedule_delay;
        }
        res
    }

    /// nRF52 SPIM EasyDMA needs bus access to read/write RAM buffers.
    fn needs_bus_tick(&self) -> bool {
        self.nrf52_pending_start
            // The whole net transfer, not just its EasyDMA moments: the walk
            // re-derives this set on MMIO writes, and a SPIM3 transfer can
            // reach its `Finishing` step from a timed CSN hold with no write
            // (or net edge) to refresh it.
            || self.nrf_net_busy()
            || self.has_external_bus_device()
            || self.selected_devices.iter().any(|selected| *selected)
    }

    /// nRF52 SPIM EasyDMA transfer engine (bare-bus / bus_tick_indices path).
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.sync_nrf52_gpio_cs(bus);
        self.poll_external_bus_devices();
        if self.nrf52_pending_start {
            self.do_nrf52_easydma(bus);
        }
        if self.nrf_net_needs_bus() {
            self.nrf_net_dma(bus);
        }
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.poll_external_bus_devices();
        self.tick_elapsed(1)
    }

    /// Legacy-walk clocking (non-event-scheduler builds): advance the bit
    /// engine by the elapsed peripheral-clock cycles. The event-scheduler
    /// build never calls this for the SPI (the walk skips scheduler-driven
    /// peripherals), so the two clocking paths cannot double-advance.
    fn tick_elapsed(&mut self, cycles: u64) -> crate::PeripheralTickResult {
        let mut irq = false;
        let mut fired: Vec<u32> = Vec::new();

        // ── nRF52 SPIM on a world `gpio_net`: the bit engine's due steps ─────
        if self.is_nrf() {
            self.nrf_net_tick();
        }
        // ── nRF52 SPIM: raise IRQ for any enabled+pending EVENTS ─────────────
        if let SpiRegs::Nrf52(r) = &self.regs {
            if r.map == NrfSpimMap::Nrf54l {
                // Same events, different enable bits AND different event
                // addresses — `fired_events` carries the offset a debugger will
                // look for, so it must be this map's offset, not the nRF52 one.
                if r.events_started != 0 && r.inten & NRF54L_INTEN_STARTED != 0 {
                    irq = true;
                    fired.push(0x100);
                }
                if r.events_stopped != 0 && r.inten & NRF54L_INTEN_STOPPED != 0 {
                    irq = true;
                    fired.push(0x104);
                }
                if r.events_end != 0 && r.inten & NRF54L_INTEN_END != 0 {
                    irq = true;
                    fired.push(0x108);
                }
                if r.events_endrx != 0 && r.inten & NRF54L_INTEN_DMA_RX_END != 0 {
                    irq = true;
                    fired.push(0x14C);
                }
                if r.events_endtx != 0 && r.inten & NRF54L_INTEN_DMA_TX_END != 0 {
                    irq = true;
                    fired.push(0x168);
                }
                return crate::PeripheralTickResult {
                    irq,
                    fired_events: fired,
                    ..Default::default()
                };
            }
            // Check each event against its INTEN bit.
            if r.events_stopped != 0 && r.inten & INTEN_STOPPED != 0 {
                irq = true;
                fired.push(0x104);
            }
            if r.events_endrx != 0 && r.inten & INTEN_ENDRX != 0 {
                irq = true;
                fired.push(0x110);
            }
            if r.events_end != 0 && r.inten & INTEN_END != 0 {
                irq = true;
                fired.push(0x118);
            }
            if r.events_endtx != 0 && r.inten & INTEN_ENDTX != 0 {
                irq = true;
                fired.push(0x120);
            }
            if r.events_started != 0 && r.inten & nrf_net::INTEN_STARTED != 0 {
                irq = true;
                fired.push(0x14C);
            }
            return crate::PeripheralTickResult {
                irq,
                fired_events: fired,
                ..Default::default()
            };
        }

        // ── STM32 SPI: bit engine clocks the frame on the wire ───────────────
        if self.frame.is_some() && self.stm32_advance_units(cycles) {
            irq = true; // TXEIE at frame completion
        }
        // ── World `gpio_net`: an interrupt a net edge raised ─────────────────
        if std::mem::take(&mut self.net_irq_pending) {
            irq = true;
        }
        // ── STM32 DMA requests (TXDMAEN / RXDMAEN) ───────────────────────────
        let dma_signals = self.stm32_take_dma_signals();
        // H5 "SPI v3" has no frame engine: its interrupt line is the level
        // SR & IER, including EOT (which HAL_SPI_IRQHandler needs to finish
        // a DMA transfer) and RXP (polling/interrupt transfers).
        if self.h5_irq_pending() {
            irq = true;
        }

        // ── H5 "SPI v3": publish any buffered narration burst ────────────────
        // Under `event-scheduler` the walk skips this model and the event chain
        // owns publication; without the feature the walk still runs and this is
        // where an H5 burst reaches the pads. Inert — one `is_empty` check — on
        // every bus that has not routed an H5 SPI pad.
        // Publish only once the wire has actually had time to carry the burst.
        // Testing that FIRST matters: a flush attempted before then comes back
        // `LevelsOnly` having built (and thrown away) the whole narration plan,
        // and the walk would repeat that every single tick. See
        // `h5_wire_pending_cycles`.
        if !self.h5_wire_words.is_empty() && self.h5_wire_pending_cycles() == 0 {
            self.h5_wire_flush(true);
        }

        // ── EFR32 Series-2 USART: publish any buffered SPI burst ─────────────
        // The bytes are held until the timeline has room for them (see
        // `efr32_wire_flush`), and firmware that writes a burst and then stops
        // writing would otherwise never trigger a retry. Inert — one
        // `is_empty` check — on every other layout.
        if !self.efr32_wire_bytes.is_empty() {
            self.efr32_wire_flush();
        }

        crate::PeripheralTickResult {
            irq,
            cycles: 0,
            dma_signals: (!dma_signals.is_empty()).then_some(dma_signals),
            ..Default::default()
        }
    }

    /// The H5 narrator needs "now" to pace a burst against the wire. Attached
    /// by the registration choke (`add_peripheral` / `push_peripheral`), so a
    /// `from_config` bus always has one; a hand-built test bus does not, and
    /// [`Self::h5_wire_flush`] publishes nothing rather than guessing a cycle.
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.h5_clock = Some(clock);
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for dev in self.attached_devices.iter_mut() {
            if let Some(si) = dev.as_sim_input_mut() {
                if f(si) {
                    return true;
                }
            }
        }
        false
    }

    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for dev in &self.attached_devices {
            crate::inspect::visit_spi_device(&**dev, f);
        }
    }
}

impl Spi {
    /// After an nRF register write: hand `TASKS_START` / `TASKS_STOP` to the
    /// net engine when the SPIM is on a world `gpio_net`, and re-state its
    /// line drives.
    fn nrf_after_write(&mut self, reg_offset: u64, value: u32, start_triggered: bool) {
        let net = self.nrf_net_mode();
        if start_triggered {
            if net {
                self.nrf_net_start();
            } else {
                self.nrf52_pending_start = true;
            }
        }
        if net {
            if reg_offset == 0x014 && value != 0 {
                self.nrf_net_stop();
            }
            self.nrf_net_apply_drive();
        } else if self.nrf_net_needs_bus() || self.nrf_net_busy() {
            self.nrf_net_disable();
        }
    }

    fn sync_nrf52_gpio_cs(&mut self, bus: &dyn Bus) {
        if !matches!(self.regs, SpiRegs::Nrf52(_)) {
            return;
        }
        for (index, device) in self.attached_devices.iter_mut().enumerate() {
            let pin = device.cs_pin();
            if pin.is_empty() {
                continue;
            }
            let selected = bus.read_gpio_output_by_label(pin) == Some(false);
            match (self.selected_devices[index], selected) {
                (false, true) => device.cs_select(),
                (true, false) => device.cs_release(),
                _ => {}
            }
            self.selected_devices[index] = selected;
        }
    }
    /// Nordic **legacy SPI** (Arduino `SPI.transfer`): write `TXD` with
    /// `ENABLE=1` clocks one byte through attached devices, latches MISO into
    /// `RXD`, and raises `EVENTS_READY` so firmware's busy-wait exits.
    ///
    /// This is a different register file from SPIM EasyDMA (`ENABLE=7`). The
    /// Arduino nRF5 SPI library uses the legacy peripheral; without this path
    /// `while (!EVENTS_READY)` hangs forever under sim.
    fn nrf52_legacy_spi_txd(&mut self, mosi: u8) {
        let mut miso = 0u8;
        if !self.attached_devices.is_empty() {
            for dev in &mut self.attached_devices {
                let resp = dev.transfer(mosi);
                if resp != 0 {
                    miso = resp;
                }
            }
        } else if self.loopback {
            miso = mosi;
        }
        if let SpiRegs::Nrf52(r) = &mut self.regs {
            r.legacy_rxd = u32::from(miso);
            r.events_ready = 1;
        }
    }

    /// nRF52 SPIM EasyDMA engine shared by `tick_with_bus` and `on_event`.
    ///
    /// Reads TXD.MAXCNT bytes from RAM at TXD.PTR, clocks each through the
    /// attached `SpiDevice` (or uses ORC when TXD is exhausted but RXD still
    /// has capacity), writes received bytes to RAM at RXD.PTR up to
    /// RXD.MAXCNT, then sets EVENTS_ENDTX / EVENTS_ENDRX / EVENTS_END and
    /// updates TXD.AMOUNT / RXD.AMOUNT.
    fn do_nrf52_easydma(&mut self, bus: &mut dyn Bus) {
        if !self.nrf52_pending_start {
            return;
        }
        self.nrf52_pending_start = false;

        let (txd_ptr, txd_maxcnt, rxd_ptr, rxd_maxcnt, orc) = if let SpiRegs::Nrf52(r) = &self.regs
        {
            (
                r.txd_ptr as u64,
                r.txd_maxcnt as usize,
                r.rxd_ptr as u64,
                r.rxd_maxcnt as usize,
                (r.orc & 0xFF) as u8,
            )
        } else {
            return;
        };

        // ── Hardware D/C and hardware chip select (nRF54L only) ─────────────
        //
        // The nRF54L SPIM drives the display's data/command line ITSELF from
        // `PSEL.DCX`, holding it LOW for the first `DCXCNT` bytes of the
        // transfer and HIGH for the rest (SVD GLOBAL_SPIM00.DCXCNT: "the number
        // of command bytes preceding the data bytes"). That is a real
        // difference from the nRF52-era boards, where D/C is a plain GPIO the
        // firmware toggles between transfers and the bus latches through
        // `dc_source`. A panel wired to PSEL.DCX declares NO `dc_pin`, so the
        // GPIO latch skips it and this is the only writer of its D/C level.
        //
        // `PSEL.CSN` is the same story for chip select: with it connected the
        // controller asserts CS for the duration of the transfer, so a device
        // on this bus is selected by the hardware rather than by a firmware
        // pin write, and the `selected_devices` gate must not veto it.
        //
        // ⚠️ `dcx_live` is PSEL.DCX being CONNECTED — never `DCXCNT > 0`.
        // DCXCNT = 0 is a legal, common setting meaning "no command bytes, this
        // whole transfer is data", and it must drive D/C HIGH for every byte.
        // Treating it as "D/C not in use" instead leaves the line at whatever
        // the previous transfer left it at, which is LOW after any command —
        // so a pixel burst would be fed to the panel as commands, and only
        // after a transfer that happened to end high would it ever look right.
        let (dcx_bytes, dcx_live, hw_csn) = match &self.regs {
            SpiRegs::Nrf52(r) if r.map == NrfSpimMap::Nrf54l => (
                r.dcxcnt as usize,
                r.psel_dcx & 0x8000_0000 == 0,
                r.psel_csn & 0x8000_0000 == 0,
            ),
            _ => (0, false, false),
        };
        if hw_csn {
            for dev in self.attached_devices.iter_mut() {
                dev.cs_select();
            }
        }

        // Determine the total number of byte-cycles to run: whichever
        // descriptor is larger drives the clock count; the smaller one
        // pads with ORC (TX side) or discards (RX side that is full).
        let n_clocks = txd_maxcnt.max(rxd_maxcnt);

        let mut txd_amount: u32 = 0;
        let mut rxd_amount: u32 = 0;
        // MOSI bytes as they leave, for the pad narration below. Collected only
        // when a GPIO port routes this controller's pads and the burst is short
        // enough to draw — see `nrf52_wire_flush`.
        let mut mosi_wire: Vec<u8> = if self.lines.is_some() && n_clocks <= NRF52_WIRE_BYTE_CAP {
            Vec::with_capacity(n_clocks)
        } else {
            Vec::new()
        };

        for i in 0..n_clocks {
            // Read MOSI byte: TX buffer while available, else ORC.
            let mosi: u8 = if i < txd_maxcnt {
                let at = txd_ptr + i as u64;
                crate::with_device_write_bypass(bus, |bus| bus.read_u8(at)).unwrap_or(0)
            } else {
                orc
            };

            if i < txd_maxcnt {
                txd_amount += 1;
            }
            if mosi_wire.capacity() > 0 {
                // ORC padding is on the wire too: the clock keeps running past
                // TXD.MAXCNT to shift the extra RX bytes in, and the over-read
                // character is what MOSI carries while it does.
                mosi_wire.push(mosi);
            }

            // Clock the byte through the attached device (or loopback /
            // no-device — mirrors MOSI back).
            let miso: u8 = if !self.attached_devices.is_empty() {
                let mut resp: u8 = 0;
                // Hardware D/C: LOW for the first DCXCNT bytes, HIGH after.
                let dc_high = i >= dcx_bytes;
                for (index, dev) in self.attached_devices.iter_mut().enumerate() {
                    if !hw_csn && !dev.cs_pin().is_empty() && !self.selected_devices[index] {
                        continue;
                    }
                    if dcx_live {
                        dev.set_dc_level(dc_high);
                    }
                    let r = dev.transfer(mosi);
                    if r != 0 {
                        resp = r;
                    }
                }
                resp
            } else if self.loopback {
                mosi
            } else {
                0
            };

            // Write MISO byte to RX buffer if there is still capacity.
            if i < rxd_maxcnt {
                let at = rxd_ptr + i as u64;
                let _ = crate::with_device_write_bypass(bus, |bus| bus.write_u8(at, miso));
                rxd_amount += 1;
            }
        }

        if hw_csn {
            // CS rises at the end of the transfer. A panel flushes its pending
            // command on the rising edge, so this is not cosmetic.
            for dev in self.attached_devices.iter_mut() {
                dev.cs_release();
            }
        }

        // Update AMOUNT registers and fire completion events.
        if let SpiRegs::Nrf52(r) = &mut self.regs {
            r.txd_amount = txd_amount;
            r.rxd_amount = rxd_amount;
            // HW fires ENDTX, ENDRX, then END (PS §6.30 sequence).
            r.events_endtx = 1;
            r.events_endrx = 1;
            r.events_end = 1;
            // STARTED is decoded on the nRF54L map only; setting it on the
            // nRF52 map would publish an event that map cannot read back.
            if r.map == NrfSpimMap::Nrf54l {
                r.events_started = 1;
            }
        }
        // The whole buffer has clocked out as far as this model is concerned,
        // so this is where the burst becomes narratable.
        self.nrf52_wire_flush(&mosi_wire);
    }
}

#[path = "spi_net.rs"]
mod net;

#[path = "spi_nrf_net.rs"]
mod nrf_net;

#[cfg(test)]
#[path = "spi_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "spi_efr32s2_spi_tests.rs"]
mod efr32s2_spi_tests;

#[cfg(test)]
#[path = "spi_dma_tests.rs"]
mod dma_tests;
