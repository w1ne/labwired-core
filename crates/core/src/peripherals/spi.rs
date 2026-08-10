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
use std::any::Any;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use crate::peripherals::pad_lines::PadLines;
use crate::peripherals::spi_waveform::{NarrationFit, SpiFraming, SpiNarrator};

/// Trait implemented by simulated SPI devices (peripherals attached to an SPI bus).
///
/// For v1, CS-pin-aware routing is not implemented: all transfers are broadcast
/// to every attached device and the first non-zero MISO byte wins.  This is
/// correct for single-device labs (MAX31855 alone).  CS-aware routing is noted
/// as a Phase 2 follow-up.
/// How an attached slave latches the SPI wire — the opt-in fidelity switch.
///
/// [`Byte`](Self::Byte) is the default and the only mode any device had before
/// edge sampling existed: the engine consults the device ONCE per frame at the
/// frame boundary and the answer rides MISO bit-by-bit during that frame. The
/// device never sees a clock edge, so a CPOL/CPHA mismatch between master and
/// slave exchanges perfectly good bytes — the documented honest limit of the
/// byte-level contract.
///
/// [`Edge`](Self::Edge) opts a device into edge-accurate sampling: it declares
/// the mode ITS OWN silicon is strapped for, and the bit engine then latches
/// MOSI into it, and clocks MISO out of it, on the physical SCK edges that mode
/// selects (see [`Spi::edge_slave_capture`] / [`Spi::edge_miso_wire`]). A
/// master/slave mode mismatch then corrupts data the way real silicon does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpiSampling {
    /// Frame-boundary byte exchange. The default; costs nothing per bit.
    #[default]
    Byte,
    /// Edge-accurate slave, strapped for this CPOL/CPHA.
    Edge { cpol: bool, cpha: bool },
}

impl SpiSampling {
    /// Build an edge-sampling mode from the usual SPI mode number
    /// (bit 1 = CPOL, bit 0 = CPHA): 0 → (0,0) … 3 → (1,1).
    pub const fn edge_mode(mode: u8) -> Self {
        Self::Edge {
            cpol: mode & 0b10 != 0,
            cpha: mode & 0b01 != 0,
        }
    }
}

pub trait SpiDevice: Send {
    /// How this device latches the wire. Default [`SpiSampling::Byte`] — the
    /// pre-existing frame-boundary contract, so every device model that does
    /// not override this compiles and behaves exactly as before.
    ///
    /// Honoured by the STM32 classic/FIFO bit engine in this module and by the
    /// ESP32-C3 GP-SPI controller, which share ONE edge model
    /// ([`edge_slave_capture`] / [`edge_miso_wire`]). The remaining
    /// controllers (ESP32 classic, ESP32-S3, nRF52 SPIM, STM32H5 SPIv3,
    /// Kinetis DSPI) exchange whole bytes; attaching an opt-in device to one of
    /// them is REJECTED at config time, naming the controller, rather than
    /// silently ignored (see `SystemBus::attach_spi_device`).
    fn sampling(&self) -> SpiSampling {
        SpiSampling::Byte
    }
    fn needs_external_bus_poll(&self) -> bool {
        false
    }
    fn component_id(&self) -> Option<&str> {
        None
    }
    fn attach_can_bus(
        &mut self,
        _tx: Sender<crate::network::CanFrame>,
        _rx: Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("SPI device is not a CAN controller")
    }
    fn poll_external_bus(&mut self) {}
    /// Called when the CS line goes low (chip is selected).
    fn cs_select(&mut self) {}
    /// Called when the CS line goes high (chip is released — flush state).
    fn cs_release(&mut self) {}
    /// SPI is full-duplex: master sends `mosi_byte`, device returns its current MISO byte.
    /// On read-only devices like MAX31855, `mosi_byte` is ignored.
    fn transfer(&mut self, mosi_byte: u8) -> u8;
    /// CS pin label this device is wired to (e.g. "PA4" or numeric pin ID). Used by the bus
    /// dispatcher to pick which device responds when the firmware drives a particular CS line.
    fn cs_pin(&self) -> &str;

    /// What this device can show of itself — its own inspect evidence.
    ///
    /// The ONE place a a SPI device's artifacts are decided is the model
    /// itself, next to the buffers it owns. Default: nothing, which is correct
    /// for a sensor with no display surface and honest for anything else —
    /// absent means "this engine has nothing to show", never "the screen was
    /// blank". See [`crate::inspect::DeviceEvidence`] for why this is not a
    /// central match on concrete types.
    ///
    /// Implementations must read the model's REAL buffer and synthesize
    /// nothing; a panel that was never painted reports zero.
    fn artifacts(
        &self,
        _id: &str,
        _opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        Vec::new()
    }
    /// Data/Command (D/C) pin label this device observes, if any (e.g. "PB6").
    ///
    /// Displays like the Nokia 5110 (PCD8544) distinguish command bytes from
    /// pixel-data bytes by the level of a dedicated GPIO line rather than by
    /// byte semantics. When this returns `Some(pin)`, the bus latches that
    /// pin's current output level into the device via [`set_dc_level`] after
    /// each MMIO write, so the value is current by the time the firmware
    /// writes the SPI data register. Default `None` → the bus does no latching
    /// and the device infers framing from the protocol (ILI9341 / SSD1680).
    ///
    /// [`set_dc_level`]: SpiDevice::set_dc_level
    fn dc_pin(&self) -> Option<&str> {
        None
    }
    /// Latched level of the [`dc_pin`](SpiDevice::dc_pin) at transfer time,
    /// pushed by the bus. No-op for devices that do not observe a D/C line.
    fn set_dc_level(&mut self, _level: bool) {}
    /// Resolved `(ODR address, bit)` of the D/C line. The bus computes this
    /// once at install time (from [`dc_pin`](SpiDevice::dc_pin)) and records it
    /// via [`set_dc_source`]; thereafter the bus reads that GPIO output bit
    /// just before each transfer and pushes the level via [`set_dc_level`].
    /// Default `None` → no D/C latching.
    ///
    /// [`set_dc_source`]: SpiDevice::set_dc_source
    fn dc_source(&self) -> Option<(u64, u8)> {
        None
    }
    /// Bus-side setter recording the resolved D/C `(ODR address, bit)`.
    fn set_dc_source(&mut self, _odr_addr: u64, _bit: u8) {}
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    /// Runtime-drivable view of this device, if it accepts simulated input.
    /// Same contract as the hook on `I2cDevice`: input devices override it so
    /// the generic [`crate::Machine::set_input`] resolver can reach them
    /// without a downcast. Default `None` = not an input device.
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        None
    }
    /// Binary mid-flight snapshot for runtime resume. Default empty;
    /// override for stateful devices (e-paper panels with framebuffers,
    /// thermocouples with cached temperatures, etc.).
    fn runtime_snapshot(&self) -> Vec<u8> {
        Vec::new()
    }
    fn restore_runtime_snapshot(&mut self, _bytes: &[u8]) -> crate::SimResult<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpiRegisterLayout {
    #[default]
    Stm32,
    /// STM32 families with a TX/RX FIFO + CR2.DS data-size field (L4/F7/G4/…).
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
    /// NXP Kinetis **DSPI** (KW41Z `SPI0/SPI1`) — FIFO master with MCR / TCR /
    /// CTAR / SR / PUSHR / POPR. A frame is transmitted by writing PUSHR (the
    /// low 16 bits are the data, the high bits select PCS / CONT / EOQ); the
    /// `fsl_dspi` blocking path polls SR.TFFF before the push and SR.TCF after.
    /// See [`KinetisDspiRegs`].
    KinetisDspi,
}

impl FromStr for SpiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32" | "stm32f1" | "stm32f4" | "stm32v2" => Ok(Self::Stm32),
            "stm32_fifo" | "stm32l4" | "stm32f7" | "stm32g4" => Ok(Self::Stm32Fifo),
            // H5 carries the H7-lineage "SPI v3" IP, not the L4/F7 FIFO map.
            "stm32h5" => Ok(Self::Stm32H5),
            "nrf52" | "nrf52_spim" | "nrf_spim" | "nordic" => Ok(Self::Nrf52Spim),
            "kinetis" | "dspi" | "kinetis_dspi" | "nxp_dspi" | "kw41z" => Ok(Self::KinetisDspi),
            _ => Err(format!(
                "unsupported SPI register layout '{}'; supported: stm32, stm32_fifo, stm32h5, nrf52, kinetis",
                value
            )),
        }
    }
}

/// Event token for the SPI bit engine's next-wire-transition event (STM32).
const SPI_DONE_TOKEN: u32 = 0;
/// Event token for nRF52 SPIM EasyDMA completion (delay-0 scheduler path).
const SPI_NRF52_EASYDMA_TOKEN: u32 = 1;

/// High bit marks an H5 wire-narration wakeup. The low 31 bits carry the arm
/// sequence, so a stale wakeup from a superseded arm is recognisable — and
/// because the flag is always set, an H5 token can never collide with
/// [`SPI_DONE_TOKEN`] (0) or [`SPI_NRF52_EASYDMA_TOKEN`] (1) no matter how far
/// the sequence wraps.
const SPI_H5_WIRE_TOKEN_FLAG: u32 = 0x8000_0000;

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
const SPI_LINES: &[&str] = &["SCK", "MOSI", "MISO"];

impl SpiLineLevels {
    fn new(sck_idle: bool) -> Self {
        Self {
            // MOSI/MISO idle low; SCK idles at CPOL.
            lines: std::sync::Arc::new(PadLines::new(SPI_LINES, &[sck_idle, false, false])),
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
        self.lines.set(&[sck, mosi, miso]);
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
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Nrf52SpiRegs {
    // EVENTS — HW-set only; SW may only write 0 to clear
    events_stopped: u32,
    events_endrx: u32,
    events_end: u32,
    events_endtx: u32,

    // INTEN — bit-field enabling each event's IRQ
    inten: u32,

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
}

/// `ENABLE` value that selects the SPIM personality on the shared
/// SPIM/SPIS/SPI/TWIM/TWI/TWIS window (nRF52840 PS v1.11 §6.25.6.17, p733).
const NRF52_ENABLE_SPIM: u32 = 7;
/// `CONFIG` fields (nRF52840 PS v1.11 §6.25.6.22, p737).
const NRF52_CONFIG_ORDER_LSB: u32 = 1 << 0;
const NRF52_CONFIG_CPHA: u32 = 1 << 1;
const NRF52_CONFIG_CPOL: u32 = 1 << 2;
/// Buffered MOSI bytes past which a transfer's narration is dropped rather than
/// truncated. See [`Spi::nrf52_wire_flush`].
const NRF52_WIRE_BYTE_CAP: usize = 2_048;

/// INTEN bit positions (PS §6.30 INTEN register).
/// STOPPED=1, ENDRX=4, END=6, ENDTX=8.
const INTEN_STOPPED: u32 = 1 << 1;
const INTEN_ENDRX: u32 = 1 << 4;
const INTEN_END: u32 = 1 << 6;
const INTEN_ENDTX: u32 = 1 << 8;

impl Nrf52SpiRegs {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            // TASKS read as 0 (write-only strobes on silicon)
            0x010 | 0x014 => 0,
            // EVENTS
            0x104 => self.events_stopped,
            0x110 => self.events_endrx,
            0x118 => self.events_end,
            0x120 => self.events_endtx,
            // INTEN / INTENSET / INTENCLR all mirror the inten value
            0x300 | 0x304 | 0x308 => self.inten,
            // Config
            0x500 => self.enable,
            0x508 => self.psel_sck,
            0x50C => self.psel_mosi,
            0x510 => self.psel_miso,
            0x514 => self.psel_csn,
            0x524 => self.frequency,
            0x554 => self.config,
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
    fn write_reg(&mut self, offset: u64, value: u32) -> bool {
        match offset {
            // TASKS — trigger on non-zero write
            0x010 => return value != 0, // TASKS_START: signal caller
            0x014 => {
                // TASKS_STOP: no state needed; events_stopped set by HW
            }

            // EVENTS — SW write of 1 ignored; SW write of 0 clears
            0x104 if value == 0 => self.events_stopped = 0,
            0x110 if value == 0 => self.events_endrx = 0,
            0x118 if value == 0 => self.events_end = 0,
            0x120 if value == 0 => self.events_endtx = 0,

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
/// silicon that does not have it.
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
}

impl FromStr for SpiPadMap {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "stm32h5" | "h5" | "stm32h7" | "h7" => Ok(Self::Stm32H5),
            "stm32wba" | "wba" => Ok(Self::Stm32Wba),
            "none" => Ok(Self::None),
            other => Err(anyhow::anyhow!(
                "unsupported SPI pad_map '{other}'; supported: stm32h5, stm32wba, none"
            )),
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
    /// Bytes sitting in the modelled RX FIFO (FIFO layout only).
    rx_fifo_level: u8,

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
    /// Last sampled active-low GPIO CS level for each attached device.
    #[serde(skip)]
    selected_devices: Vec<bool>,
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
            SpiRegisterLayout::Nrf52Spim => SpiRegs::Nrf52(Nrf52SpiRegs::default()),
            SpiRegisterLayout::KinetisDspi => SpiRegs::KinetisDspi(KinetisDspiRegs::default()),
        };
        Self {
            regs,
            rx_fifo,
            rx_fifo_level: 0,
            cr2_mask,
            ..Default::default()
        }
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
    ) {
        if !self.is_nrf() {
            return;
        }
        self.nrf52_claim_sck.install(claims.clone(), sck_token);
        self.nrf52_claim_mosi.install(claims.clone(), mosi_token);
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
        let (enable, psel_sck, psel_mosi) = match &self.regs {
            SpiRegs::Nrf52(r) => (r.enable & 0xF, r.psel_sck, r.psel_mosi),
            _ => return,
        };
        let live = enable == NRF52_ENABLE_SPIM;
        self.nrf52_claim_sck.update(psel_sck, live);
        self.nrf52_claim_mosi.update(psel_mosi, live);
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
                        r.cr2 = if ds < 0b0011 {
                            (value & !0x0F00) | (0b0111 << 8)
                        } else {
                            value
                        };
                    } else {
                        r.cr2 = value & cr2_mask;
                    }
                }
            }
            0x08 => {
                // SR is mostly read-only; allow clearing OVR if modelled.
                if let SpiRegs::Stm32(r) = &mut self.regs {
                    r.sr = value & 0xFFBF;
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
                    } else if fifo && self.tx_queue.len() >= 4 {
                        // FIFO parts: the 32-bit TX FIFO is full — the write
                        // is lost (RM0351 §40.4.9). Conservative frame-count
                        // bound (4 × 8-bit frames).
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
                    r.cr2 = value & 0xFFFF;
                }
            }
            0x08 => {
                // CFG1: ignored while SPE=1 (config lock); reserved bits
                // read as 0.
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    if r.cr1 & H5_CR1_SPE == 0 {
                        r.cfg1 = value & !H5_CFG1_RESERVED;
                    }
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
                // TXDR — the TX-only data engine (spec-derived; the bench
                // part had no SPI kernel clock, see Stm32H5SpiRegs docs).
                let (spe, started, master) = match &self.regs {
                    SpiRegs::Stm32H5(r) => (
                        r.cr1 & H5_CR1_SPE != 0,
                        r.cr1 & H5_CR1_CSTART != 0,
                        r.cfg2 & H5_CFG2_MASTER != 0,
                    ),
                    _ => return,
                };
                if !spe {
                    return;
                }
                if let SpiRegs::Stm32H5(r) = &mut self.regs {
                    // Any enabled TXDR write fills the TxFIFO → TXTF; TXP
                    // stays set (sim TX path is bottomless).
                    r.sr |= H5_SR_TXTF;
                }
                if started && master {
                    // One frame per TXDR access. DSIZE (CFG1[4:0]) is stored
                    // but not consumed by the TX-only engine: the low byte is
                    // broadcast, matching the v1 byte-wide device routing.
                    let mosi = (value & 0xFF) as u8;
                    // Full-duplex: every transmitted frame simultaneously
                    // clocks one IN. Silicon fills RXDR and raises SR.RXP
                    // whether or not a slave drives MISO — with nothing
                    // driving, the captured value is just the idle line level.
                    // Without this the peripheral was TX-only, and any driver
                    // that writes TXDR then waits for RXP (which is what
                    // HAL_SPI_TransmitReceive, and therefore Arduino's
                    // SPI.transfer(), does) hung forever.
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
                    if let SpiRegs::Stm32H5(r) = &mut self.regs {
                        r.rxdr = miso as u32;
                        r.sr |= H5_SR_RXP;
                    }
                    // The frame just crossed the bus. Narrate it onto the
                    // routed AF pads: this IP has no bit engine to drive them
                    // per cycle, so the wire is reconstructed from the completed
                    // transfer. Buffered here, published by `h5_wire_flush`.
                    self.h5_wire_push(value as u16);
                    if let SpiRegs::Stm32H5(r) = &mut self.regs {
                        if r.ctsize > 0 {
                            r.ctsize -= 1;
                            if r.ctsize == 0 {
                                // Frame count exhausted: EOT|TXC, start
                                // request HW-cleared. TSIZE=0 (endless mode)
                                // never reaches this — no EOT.
                                r.sr |= H5_SR_EOT | H5_SR_TXC;
                                r.cr1 &= !H5_CR1_CSTART;
                            }
                        }
                    }
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

    /// `true` for the H5/H7 "SPI v3" register file, which selects the H5
    /// alternate-function pad tables rather than the classic/FIFO ones.
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

    /// `true` for the FIFO (L4/F7/G4) flavour of the STM32 layout.
    pub(crate) fn is_fifo_layout(&self) -> bool {
        matches!(&self.regs, SpiRegs::Stm32(r) if r.fifo)
    }

    /// Get-or-create the shared line-level cell (bus wiring hands the same
    /// `Arc` to the STM32 GPIO ports carrying this SPI's AF pads).
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
                _ => false,
            };
            self.lines = Some(Arc::new(SpiLineLevels::new(cpol)));
        }
        self.lines.as_ref().unwrap().clone()
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
                    let answer = dev.transfer((slave_rx & 0xFF) as u8);
                    if answer != 0 {
                        resp = answer;
                    }
                }
                let (halves, master_rx) =
                    edge_miso_wire(&t.wire(), u16::from(resp), prev_miso, s_cpol, s_cpha);
                self.edge_hold = (
                    frame_bit(&t.wire(), mosi, t.bits - 1),
                    (halves >> (2 * u32::from(t.bits) - 1)) & 1 != 0,
                );
                (master_rx, Some(halves))
            }
            // ── Default: byte-level, unchanged ───────────────────────────────
            _ => {
                let miso = if !self.attached_devices.is_empty() {
                    let mosi_byte = (mosi & 0xFF) as u8;
                    let mut miso_byte = 0u8;
                    for dev in &mut self.attached_devices {
                        let resp = dev.transfer(mosi_byte);
                        if resp != 0 {
                            miso_byte = resp;
                        }
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
        self.frame = Some(ActiveFrame {
            t,
            mosi,
            miso,
            miso_halves,
            bit_idx: 0,
            second_half: false,
            ticks_left: t.half_ticks,
        });
        self.stm32_drive_levels();
    }

    /// Advance the wire by `units` peripheral-clock cycles. Returns `true`
    /// when a completed frame wants the TXE interrupt raised (CR2.TXEIE).
    fn stm32_advance_units(&mut self, mut units: u64) -> bool {
        let mut irq = false;
        while units > 0 {
            let Some(f) = &mut self.frame else { break };
            let step = (f.ticks_left as u64).min(units);
            f.ticks_left -= step as u32;
            units -= step;
            if f.ticks_left == 0 {
                self.stm32_segment_boundary(&mut irq);
            }
        }
        irq
    }

    /// The current half-period expired: drive the next segment, or complete
    /// the frame at derived wire time (RXNE/BSY/TXE flip HERE, not at the DR
    /// write).
    fn stm32_segment_boundary(&mut self, irq: &mut bool) {
        let Some(mut f) = self.frame.take() else {
            return;
        };
        if !f.second_half {
            f.second_half = true;
            f.ticks_left = f.t.half_ticks;
            self.frame = Some(f);
            self.stm32_drive_levels();
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
        let driven = self.loopback || !self.attached_devices.is_empty();
        let level = &mut self.rx_fifo_level;
        if let SpiRegs::Stm32(r) = &mut self.regs {
            r.dr = f.miso;
            if !rx_fifo {
                // Classic F1/F4 port: no FIFO, RXNE on every frame.
                r.sr |= 0x0001;
            } else {
                // FIFO port: RXNE follows CR2.FRXTH (bit 12). FRXTH=1 → the
                // threshold is 8 bit, so one frame asserts it; FRXTH=0 (reset)
                // → 16 bit, so a single 8-bit frame must NOT assert it. This is
                // what a real NUCLEO-L476RG reports (SR=0x0002 after TX), and
                // is why STM32duino sets FRXTH before 8-bit transfers.
                // A slave (or loopback) must actually drive MISO for the FIFO
                // to fill: a real NUCLEO-L476RG with nothing wired reports
                // SR=0x0002 after a transmit even with FRXTH=1, which is the
                // value pinned by test_nucleo_l476rg_spi_survival. Only once
                // data is genuinely present does the CR2.FRXTH threshold decide
                // whether one 8-bit frame is enough to raise RXNE (FRXTH=1) or
                // whether 16 bits are required (FRXTH=0, the reset default).
                if driven {
                    *level = level.saturating_add(1);
                    let frxth = r.cr2 & (1 << 12) != 0;
                    if frxth || *level >= 2 {
                        r.sr |= 0x0001;
                    }
                }
            }
        }
        if !self.tx_queue.is_empty() {
            // Back-to-back: the next queued frame starts on the very next
            // cycle (BSY stays set, TXE stays clear).
            self.stm32_start_next_frame();
            return;
        }
        self.frame = None;
        if let SpiRegs::Stm32(r) = &mut self.regs {
            r.sr &= !0x0080; // Clear BSY
            r.sr |= 0x0002; // Set TXE
            if (r.cr2 & (1 << 7)) != 0 {
                *irq = true; // TXEIE
            }
        }
        // Wire idles: SCK returns to CPOL (the trailing edge of the last
        // bit); MOSI/MISO hold their last driven level, like real pads.
        if let Some(lines) = &self.lines {
            lines.set(f.t.cpol, lines.mosi(), lines.miso());
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
}

impl crate::Peripheral for Spi {
    fn line_names(&self) -> &'static [&'static str] {
        SPI_LINES
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_ref().map(|levels| &**levels.pad_lines())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = match &self.regs {
            SpiRegs::Nrf52(r) => r.read_reg(reg_offset),
            SpiRegs::KinetisDspi(r) => r.read_reg(reg_offset),
            SpiRegs::Stm32H5(r) => r.read_reg(reg_offset),
            // Widen u16→u32 before the shift: byte accesses at offsets 2/3 read
            // the upper byte of the next halfword; `(u16 as u32) >> 16` is 0
            // without an overflow panic under the CI release profile.
            SpiRegs::Stm32(r) => r.read_reg(reg_offset) as u32,
        };
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        if let SpiRegs::Nrf52(_) = &self.regs {
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
            if start_triggered {
                self.nrf52_pending_start = true;
            }
            // ENABLE (0x500) and PSEL.SCK/MOSI (0x508/0x50C) all move which pad
            // this controller's wire reaches. Republishing on every nRF write
            // rather than only on those three offsets keeps the ONE call site:
            // `update` is two comparisons when nothing moved.
            self.sync_nrf_pin_claims();
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
        if let SpiRegs::Nrf52(_) = &self.regs {
            let reg_offset = offset & !3;
            let start_triggered = if let SpiRegs::Nrf52(r) = &mut self.regs {
                r.write_reg(reg_offset, value)
            } else {
                false
            };
            if start_triggered {
                self.nrf52_pending_start = true;
            }
            // See the byte-write path: one call site for every pad-moving write.
            self.sync_nrf_pin_claims();
            return Ok(());
        }
        // STM32H5: 32-bit registers must be written atomically — a word write
        // to TXDR is ONE frame (byte-splitting would transmit four), and IFCR
        // (write-1-to-clear) must see the full mask in a single access.
        if let SpiRegs::Stm32H5(_) = &self.regs {
            self.write_stm32h5_reg(offset & !3, value);
            return Ok(());
        }
        // Kinetis DSPI: PUSHR is one 32-bit frame push — must be atomic (byte
        // splitting would transmit spurious frames), so handle the word here.
        if let SpiRegs::KinetisDspi(_) = &self.regs {
            self.write_kinetis_dspi_reg(offset & !3, value);
            return Ok(());
        }
        // STM32 default: four byte writes.
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write(offset + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write(offset + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
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
            if (offset & !3) == 0x20 {
                self.write_stm32h5_reg(0x20, value as u32);
            } else {
                self.write(offset, (value & 0xFF) as u8)?;
                self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
            }
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
            if fifo && ds <= 0b0111 {
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
        self.anchor_tick = tick_now;
        if self.frame.is_some() {
            self.stm32_advance_units(delta);
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
        // nRF52 SPIM EasyDMA: delay-0 completion (next cycle after TASKS_START).
        if self.nrf52_pending_start {
            return vec![(0, SPI_NRF52_EASYDMA_TOKEN)];
        }
        if self.frame.is_some() && !self.scheduled {
            self.scheduled = true;
            return vec![(
                self.stm32_next_transition_ticks().saturating_sub(1),
                SPI_DONE_TOKEN,
            )];
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
        Vec::new()
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token == SPI_NRF52_EASYDMA_TOKEN {
            if self.nrf52_pending_start {
                self.do_nrf52_easydma(bus);
            }
            return crate::sched::EventResult::default();
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
            return crate::sched::EventResult::default();
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
        self.anchor_tick = now;
        if self.stm32_advance_units(delta) {
            res.raise_own_irq = true; // TXEIE at frame completion
        }
        if self.frame.is_some() {
            res.reschedule_delay = Some(self.stm32_next_transition_ticks());
            self.scheduled = true;
        }
        res
    }

    /// nRF52 SPIM EasyDMA needs bus access to read/write RAM buffers.
    fn needs_bus_tick(&self) -> bool {
        self.nrf52_pending_start
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

        // ── nRF52 SPIM: raise IRQ for any enabled+pending EVENTS ─────────────
        if let SpiRegs::Nrf52(r) = &self.regs {
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

        crate::PeripheralTickResult {
            irq,
            cycles: 0,
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
                bus.read_u8(txd_ptr + i as u64).unwrap_or(0)
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
                for (index, dev) in self.attached_devices.iter_mut().enumerate() {
                    if !dev.cs_pin().is_empty() && !self.selected_devices[index] {
                        continue;
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
                let _ = bus.write_u8(rxd_ptr + i as u64, miso);
                rxd_amount += 1;
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
        }
        // The whole buffer has clocked out as far as this model is concerned,
        // so this is where the burst becomes narratable.
        self.nrf52_wire_flush(&mosi_wire);
    }
}

#[cfg(test)]
mod tests {
    /// The named accessors index `PadLines` by `SpiSignal as usize`. Reordering
    /// either the enum or `SPI_LINES` alone would silently publish MOSI's level
    /// on the SCK lane — a waveform that looks plausible and is wrong.
    #[test]
    fn spi_line_order_matches_signal_discriminants() {
        use super::{SpiSignal, SPI_LINES};
        assert_eq!(SPI_LINES[SpiSignal::Sck as usize], "SCK");
        assert_eq!(SPI_LINES[SpiSignal::Mosi as usize], "MOSI");
        assert_eq!(SPI_LINES[SpiSignal::Miso as usize], "MISO");
        assert_eq!(SPI_LINES.len(), 3);
    }

    use super::{Spi, SpiDevice, SpiRegisterLayout};
    use crate::Peripheral;

    /// SPI slave that records every byte it receives.
    struct Capture {
        rx: Vec<u8>,
    }
    impl SpiDevice for Capture {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.rx.push(mosi);
            0
        }
        fn cs_pin(&self) -> &str {
            ""
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }
        fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
            Some(self)
        }
    }

    fn captured(spi: &Spi) -> Vec<u8> {
        spi.attached_devices[0]
            .as_any()
            .unwrap()
            .downcast_ref::<Capture>()
            .unwrap()
            .rx
            .clone()
    }

    /// Clock the bit engine to completion (DR writes no longer complete
    /// instantly — the frame is stretched over simulated cycles).
    fn run_engine(spi: &mut Spi) {
        for _ in 0..1_000_000 {
            if !spi.transfer_active() {
                return;
            }
            spi.tick_elapsed(8);
        }
        panic!("STM32 SPI bit engine did not complete");
    }

    /// FIFO-family SPI: a 16-bit DR write at DS=8 packs TWO frames — the
    /// silicon behaviour that broke the real Nokia 5110 panel. The second
    /// frame clocks back-to-back after the first on the wire.
    #[test]
    fn fifo_packs_u16_dr_write_into_two_frames() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo);
        spi.push_device(Box::new(Capture { rx: Vec::new() }));
        spi.write(0x00, 0x40).unwrap(); // CR1: SPE
        spi.write_u16(0x0C, 0x00AB).unwrap(); // 16-bit DR write, DS=8 (reset 0x0700)
        run_engine(&mut spi);
        assert_eq!(
            captured(&spi),
            vec![0xAB, 0x00],
            "DS≤8 + 16-bit DR ⇒ 2 frames"
        );
    }

    /// The correct 8-bit DR access sends exactly one frame, even on FIFO parts.
    #[test]
    fn fifo_u8_dr_write_is_one_frame() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo);
        spi.push_device(Box::new(Capture { rx: Vec::new() }));
        spi.write(0x00, 0x40).unwrap();
        spi.write(0x0C, 0xAB).unwrap(); // 8-bit DR write
        run_engine(&mut spi);
        assert_eq!(captured(&spi), vec![0xAB], "8-bit DR ⇒ 1 frame");
    }

    /// Non-FIFO STM32 (F1/F4) does NOT pack: a 16-bit DR write is one frame,
    /// so the F103 ILI9341 lab (which writes DR as u16) is unaffected.
    #[test]
    fn plain_stm32_does_not_pack() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32);
        spi.push_device(Box::new(Capture { rx: Vec::new() }));
        spi.write(0x00, 0x40).unwrap();
        spi.write_u16(0x0C, 0x00AB).unwrap();
        run_engine(&mut spi);
        assert_eq!(captured(&spi), vec![0xAB], "non-FIFO ⇒ 1 frame");
    }

    #[test]
    fn test_spi_transfer_timing() {
        let mut spi = Spi::new();
        // Enable SPI + BR=1 (f_pclk/4): (1<<6) | (1<<3) = 0x48.
        spi.write(0x00, 0x48).unwrap();

        // Reset SR has TXE set (bit 1).
        assert_ne!(spi.read(0x08).unwrap() & 0x02, 0);

        // Write DR -> start transfer.
        spi.write(0x0C, 0xAA).unwrap();
        let sr = spi.read(0x08).unwrap();
        assert_ne!(sr & 0x80, 0, "BSY set during transfer");
        assert_eq!(sr & 0x02, 0, "TXE cleared while shifting");

        // BR=1 -> divider=4 -> 8 bits * 4 = 32 ticks.
        for _ in 0..31 {
            spi.tick();
            assert_ne!(spi.read(0x08).unwrap() & 0x80, 0, "still busy mid-transfer");
        }

        spi.tick();
        let sr = spi.read(0x08).unwrap();
        assert_eq!(sr & 0x80, 0, "BSY cleared after transfer");
        assert_ne!(sr & 0x02, 0, "TXE set after transfer");
        // Full-duplex master: the receive ALWAYS completes. Silicon samples the
        // MISO line every frame and asserts RXNE when the RX buffer fills, slave
        // or no slave — with nothing driving, the captured value is simply the
        // idle line level (0x00 here), which is data, not a missing event.
        //
        // This assertion previously read `RXNE NOT set without a slave`, pinning
        // the opposite. That was wrong about the hardware and had a real cost:
        // any polling driver that writes DR then waits for RXNE — which is what
        // HAL_SPI_TransmitReceive and therefore Arduino's SPI.transfer() do —
        // hung forever on an unpopulated bus. Corrected when the Arduino
        // conformance sketch on F401 hung in SPI.transfer().
        // CLASSIC (F1/F4) port — no RX FIFO, so a completed full-duplex frame
        // always asserts RXNE, slave or no slave: silicon samples MISO every
        // frame and the captured value is simply the idle level.
        //
        // This is the opposite of the FIFO port (L4/F7/G4), where RXNE follows
        // CR2.FRXTH and a single 8-bit frame at the reset threshold (16 bit)
        // leaves RXNE clear — verified on a real NUCLEO-L476RG, SR=0x0002.
        // Both behaviours are now modelled; do not "unify" them.
        assert_ne!(sr & 0x01, 0, "classic port sets RXNE on every frame");
        assert_eq!(
            spi.read(0x0C).unwrap(),
            0x00,
            "DR holds the idle MISO level when no slave drives"
        );
    }

    /// Analytic wire time: a frame completes at EXACTLY `bits × 2^(BR+1)`
    /// peripheral-clock cycles, for two BR settings (and 16-bit DFF frames on
    /// the classic port take twice the clocks of 8-bit ones).
    #[test]
    fn frame_completes_at_exact_derived_cycle_for_two_br_settings() {
        // (CR1 BR bits, expected cycles for an 8-bit frame)
        for (br, expected) in [(0u16, 8 * 2u64), (4u16, 8 * 32u64)] {
            let mut spi = Spi::new();
            spi.write_u16(0x00, (1 << 6) | (br << 3)).unwrap(); // SPE | BR
            spi.write(0x0C, 0xA5).unwrap();
            let mut cycles = 0u64;
            while spi.transfer_active() {
                spi.tick_elapsed(1);
                cycles += 1;
                assert!(cycles < 1_000_000, "engine never completed");
            }
            assert_eq!(
                cycles, expected,
                "BR={br}: 8-bit frame must complete at bits × 2^(BR+1) cycles"
            );
        }
        // Classic 16-bit frames (CR1.DFF): twice the clocks at the same BR.
        let mut spi = Spi::new();
        spi.write_u16(0x00, (1 << 6) | (1 << 3) | (1 << 11))
            .unwrap(); // SPE|BR=1|DFF
        spi.write_u16(0x0C, 0xBEEF).unwrap();
        let mut cycles = 0u64;
        while spi.transfer_active() {
            spi.tick_elapsed(1);
            cycles += 1;
            assert!(cycles < 1_000_000, "engine never completed");
        }
        assert_eq!(cycles, 16 * 4, "DFF frame = 16 bits × 2^(BR+1) cycles");
    }

    /// Mode-3 + LSBFIRST wire shape: SCK idles HIGH (CPOL=1), data is driven
    /// on the leading (falling) edge and sampled on the trailing (rising)
    /// edge (CPHA=1), and the bit order is LSB first. Decoding the MOSI line
    /// at every SCK rising edge must reproduce the written byte.
    #[test]
    fn mode3_lsbfirst_waveform_samples_on_trailing_edge() {
        let mut spi = Spi::new();
        let lines = spi.line_levels_arc();
        // CR1: SPE | CPOL | CPHA | LSBFIRST, BR=0 (half-period = 1 cycle).
        spi.write_u16(0x00, (1 << 6) | (1 << 1) | 1 | (1 << 7))
            .unwrap();
        assert!(lines.sck(), "idle SCK level must be CPOL = 1");

        spi.write(0x0C, 0xB4).unwrap();
        let mut prev = lines.sck();
        let mut bits = Vec::new();
        for _ in 0..16 {
            spi.tick_elapsed(1);
            let sck = lines.sck();
            if sck && !prev {
                bits.push(lines.mosi()); // sample on the trailing (rising) edge
            }
            prev = sck;
        }
        assert!(!spi.transfer_active(), "16 half-periods complete the frame");
        assert!(lines.sck(), "SCK returns to the CPOL idle level");
        assert_eq!(bits.len(), 8, "8 trailing edges per 8-bit frame");
        let byte = bits
            .iter()
            .enumerate()
            .fold(0u8, |acc, (i, &b)| acc | (u8::from(b) << i));
        assert_eq!(byte, 0xB4, "LSB-first decode at the mode-3 sample edges");
    }

    /// Full-duplex fidelity: the byte the slave answers at the frame boundary
    /// is what lands in DR when the SAME frame finishes clocking — not a byte
    /// from a previous frame, and not delivered before wire time.
    #[test]
    fn slave_answer_clocks_back_during_the_same_frame() {
        struct Sequenced {
            next: u8,
        }
        impl SpiDevice for Sequenced {
            fn transfer(&mut self, _mosi: u8) -> u8 {
                let out = self.next;
                self.next = self.next.wrapping_add(1);
                out
            }
            fn cs_pin(&self) -> &str {
                ""
            }
        }
        let mut spi = Spi::new();
        spi.push_device(Box::new(Sequenced { next: 0x51 }));
        spi.write(0x00, 0x48).unwrap(); // SPE | BR=1
        spi.write(0x0C, 0x01).unwrap();
        assert_eq!(
            spi.read(0x08).unwrap() & 0x01,
            0,
            "RXNE must not assert before the frame finishes on the wire"
        );
        run_engine(&mut spi);
        assert_eq!(spi.read(0x0C).unwrap(), 0x51, "first frame's answer");
        spi.write(0x0C, 0x02).unwrap();
        run_engine(&mut spi);
        assert_eq!(spi.read(0x0C).unwrap(), 0x52, "second frame's answer");
    }

    // ── Opt-in edge (bit-level) slave sampling ────────────────────────────────

    /// Slave that declares its own CPOL/CPHA and records the bytes the wire
    /// actually delivered to it. Its answer is a constant, independent of what
    /// it receives, so a corrupted read can only come from the wire.
    struct EdgeSlave {
        mode: u8,
        answer: u8,
        rx: Vec<u8>,
        opt_in: bool,
    }
    impl EdgeSlave {
        fn new(mode: u8, answer: u8) -> Self {
            Self {
                mode,
                answer,
                rx: Vec::new(),
                opt_in: true,
            }
        }
        /// Same device, same declared mode, but staying on the default
        /// byte-level path — the control arm that proves the corruption below
        /// comes from the opt-in and not from the test rig.
        fn byte_level(mode: u8, answer: u8) -> Self {
            Self {
                opt_in: false,
                ..Self::new(mode, answer)
            }
        }
    }
    impl SpiDevice for EdgeSlave {
        fn sampling(&self) -> super::SpiSampling {
            if self.opt_in {
                super::SpiSampling::edge_mode(self.mode)
            } else {
                super::SpiSampling::Byte
            }
        }
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.rx.push(mosi);
            self.answer
        }
        fn cs_pin(&self) -> &str {
            ""
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }
    }

    /// Clock `bytes` through a master programmed for `master_mode` against
    /// `slave`, returning `(bytes the master read back, bytes the slave got)`.
    fn exchange(slave: EdgeSlave, master_mode: u8, bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut spi = Spi::new();
        spi.push_device(Box::new(slave));
        let cpol = u16::from(master_mode & 0b10 != 0);
        let cpha = u16::from(master_mode & 0b01 != 0);
        // SPE | BR=1 | CPOL | CPHA
        spi.write_u16(0x00, (1 << 6) | (1 << 3) | (cpol << 1) | cpha)
            .unwrap();
        let mut read = Vec::new();
        for &b in bytes {
            spi.write(0x0C, b).unwrap();
            run_engine(&mut spi);
            read.push(spi.read(0x0C).unwrap());
        }
        let rx = spi.attached_devices[0]
            .as_any()
            .unwrap()
            .downcast_ref::<EdgeSlave>()
            .unwrap()
            .rx
            .clone();
        (read, rx)
    }

    /// Matched modes must round-trip EXACTLY — in all four modes, both
    /// directions. Without this the mismatch tests below would pass vacuously
    /// (a model that corrupts everything corrupts mismatches too).
    #[test]
    fn edge_slave_round_trips_when_modes_match() {
        for mode in 0..=3u8 {
            let sent = [0xA5u8, 0x3C, 0xFF, 0x01];
            let (read, rx) = exchange(EdgeSlave::new(mode, 0xB3), mode, &sent);
            assert_eq!(
                rx,
                sent.to_vec(),
                "mode {mode}: slave must latch exactly what the master sent"
            );
            assert_eq!(
                read,
                vec![0xB3; 4],
                "mode {mode}: master must read exactly what the slave answered"
            );
        }
    }

    /// CPHA mismatch, master leading: the slave presents its first MISO bit on
    /// the very edge the master latches on, so the master reads the level the
    /// pad still carried plus the slave's bits shifted down one — the classic
    /// off-by-one-bit symptom of a mode mismatch on real hardware.
    #[test]
    fn edge_slave_cpha_mismatch_shifts_the_read_back() {
        let (read, rx) = exchange(EdgeSlave::new(1, 0xB3), 0, &[0xA5, 0xA5]);
        // First frame: MISO idled low, so bit 7 is 0 and 0xB3 arrives >> 1.
        assert_eq!(read[0], 0xB3 >> 1, "0xB3 sampled half a bit period late");
        // Second frame: the pad still held the slave's last bit, which is what
        // the master latches first.
        assert_eq!(read[1], 0x80 | (0xB3 >> 1));
        assert_ne!(read[0], 0xB3, "a mode mismatch must NOT read back cleanly");
        // The MOSI direction survives this particular pairing: the slave's
        // sample edge lands on the master's bit boundary and latches the level
        // still on the pad (propagation delay), which is the outgoing bit.
        assert_eq!(rx, vec![0xA5, 0xA5]);
    }

    /// The mirror image: with the master at CPHA=1 and the slave at CPHA=0 the
    /// corruption lands on MOSI — the slave latches one edge early.
    #[test]
    fn edge_slave_cpha_mismatch_shifts_what_the_slave_receives() {
        let (read, rx) = exchange(EdgeSlave::new(0, 0xB3), 1, &[0xA5, 0x3C]);
        assert_eq!(rx[0], 0xA5 >> 1, "slave latched one edge early");
        assert_eq!(
            rx[1],
            0x80 | (0x3C >> 1),
            "the bit still on the pad from the previous frame leads"
        );
        assert_ne!(rx[0], 0xA5, "a mode mismatch must NOT deliver cleanly");
        // MISO survives this pairing (mirror of the test above).
        assert_eq!(read, vec![0xB3, 0xB3]);
    }

    /// The control arm. The SAME mismatch across the SAME device model, with
    /// the opt-in switched off, keeps exchanging clean bytes — i.e. the
    /// corruption above is the opt-in doing its job, not the rig.
    #[test]
    fn byte_level_slave_is_untouched_by_a_mode_mismatch() {
        for master_mode in 0..=3u8 {
            for slave_mode in 0..=3u8 {
                let (read, rx) = exchange(
                    EdgeSlave::byte_level(slave_mode, 0xB3),
                    master_mode,
                    &[0xA5],
                );
                assert_eq!(read, vec![0xB3], "byte-level read must not change");
                assert_eq!(rx, vec![0xA5], "byte-level delivery must not change");
            }
        }
    }

    /// The opt-in must not disturb frame timing: an edge-sampled frame takes
    /// exactly the same number of peripheral-clock cycles as a byte-level one.
    #[test]
    fn edge_sampling_does_not_change_frame_wire_time() {
        fn cycles(opt_in: bool) -> u64 {
            let mut spi = Spi::new();
            spi.push_device(Box::new(if opt_in {
                EdgeSlave::new(0, 0xB3)
            } else {
                EdgeSlave::byte_level(0, 0xB3)
            }));
            spi.write_u16(0x00, (1 << 6) | (1 << 3)).unwrap();
            spi.write(0x0C, 0xA5).unwrap();
            let mut n = 0;
            while spi.transfer_active() {
                spi.tick_elapsed(1);
                n += 1;
            }
            n
        }
        assert_eq!(cycles(true), cycles(false), "8 bits x 2^(BR+1) either way");
    }

    /// Cost SHAPE, not wall clock (that lives in `tests::bench_spi_engine`):
    /// neither path may consult the device more than once per frame. A
    /// per-bit device call would be the obvious way to make edge sampling
    /// eat CPU, and this fails the moment one appears.
    #[test]
    fn neither_path_consults_the_device_more_than_once_per_frame() {
        for opt_in in [false, true] {
            let mut spi = Spi::new();
            spi.push_device(Box::new(if opt_in {
                EdgeSlave::new(0, 0xB3)
            } else {
                EdgeSlave::byte_level(0, 0xB3)
            }));
            spi.write_u16(0x00, (1 << 6) | (1 << 3)).unwrap();
            for b in 0..4u8 {
                spi.write(0x0C, b).unwrap();
                run_engine(&mut spi);
            }
            let calls = spi.attached_devices[0]
                .as_any()
                .unwrap()
                .downcast_ref::<EdgeSlave>()
                .unwrap()
                .rx
                .len();
            assert_eq!(calls, 4, "opt_in={opt_in}: one transfer() per frame");
        }
    }

    /// The MISO pad itself carries the slave's phase: with a CPHA mismatch the
    /// line transitions half a bit period away from where the byte-level path
    /// would have put them.
    #[test]
    fn edge_sampled_miso_transitions_on_the_slave_phase() {
        fn wire(opt_in: bool) -> Vec<bool> {
            let mut spi = Spi::new();
            let lines = spi.line_levels_arc();
            spi.push_device(Box::new(if opt_in {
                EdgeSlave::new(1, 0xB3)
            } else {
                EdgeSlave::byte_level(1, 0xB3)
            }));
            // Mode 0 master, BR=0 -> one cycle per half-period.
            spi.write_u16(0x00, 1 << 6).unwrap();
            spi.write(0x0C, 0xA5).unwrap();
            let mut levels = vec![lines.miso()];
            while spi.transfer_active() {
                spi.tick_elapsed(1);
                levels.push(lines.miso());
            }
            levels
        }
        let edge = wire(true);
        let byte = wire(false);
        assert_eq!(edge.len(), byte.len(), "same frame length");
        assert_ne!(
            edge, byte,
            "the mismatched slave drives MISO on its own edges"
        );
    }

    // ── nRF52 SPIM EasyDMA unit tests ─────────────────────────────────────────

    use crate::{Bus, DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    /// Minimal flat-RAM bus for unit tests — no peripherals, just byte array.
    struct FlatRamBus {
        mem: HashMap<u64, u8>,
        gpio: HashMap<String, bool>,
        config: SimulationConfig,
    }

    impl FlatRamBus {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                gpio: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }

        fn write_slice(&mut self, base: u64, data: &[u8]) {
            for (i, &b) in data.iter().enumerate() {
                self.mem.insert(base + i as u64, b);
            }
        }

        fn read_slice(&self, base: u64, len: usize) -> Vec<u8> {
            (0..len)
                .map(|i| *self.mem.get(&(base + i as u64)).unwrap_or(&0))
                .collect()
        }
    }

    impl Bus for FlatRamBus {
        fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
        fn read_gpio_output_by_label(&self, pin: &str) -> Option<bool> {
            self.gpio.get(pin).copied()
        }
    }

    /// Helper: write a u32 to nRF SPIM registers as a single word write
    /// (matches Cortex-M STR instruction semantics used by real firmware).
    fn nrf_write_u32(spi: &mut Spi, offset: u64, value: u32) {
        spi.write_u32(offset, value).unwrap();
    }

    /// Helper: read a u32 from nRF SPIM registers via 4x byte reads.
    fn nrf_read_u32(spi: &Spi, offset: u64) -> u32 {
        let b0 = spi.read(offset).unwrap() as u32;
        let b1 = spi.read(offset + 1).unwrap() as u32;
        let b2 = spi.read(offset + 2).unwrap() as u32;
        let b3 = spi.read(offset + 3).unwrap() as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    /// Full EasyDMA transfer with no attached device and no loopback:
    /// TXD bytes are read from RAM, MISO is 0 everywhere.
    /// After tick_with_bus: EVENTS_END/ENDTX/ENDRX all 1,
    /// TXD.AMOUNT == TXD.MAXCNT, RXD.AMOUNT == RXD.MAXCNT,
    /// RXD RAM contains zeros (no device, no loopback).
    #[test]
    fn nrf52_spim_easydma_no_device_txd_and_rxd_amount() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0000;
        let rx_base: u64 = 0x2000_0100;
        let tx_data: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
        bus.write_slice(tx_base, &tx_data);

        // Configure SPIM: ENABLE=7, TXD.PTR/MAXCNT, RXD.PTR/MAXCNT.
        nrf_write_u32(&mut spi, 0x500, 7); // ENABLE = 7
        nrf_write_u32(&mut spi, 0x544, tx_base as u32); // TXD.PTR
        nrf_write_u32(&mut spi, 0x548, 4); // TXD.MAXCNT = 4
        nrf_write_u32(&mut spi, 0x534, rx_base as u32); // RXD.PTR
        nrf_write_u32(&mut spi, 0x538, 4); // RXD.MAXCNT = 4

        // TASKS_START — must not have fired events yet.
        nrf_write_u32(&mut spi, 0x010, 1);
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            0,
            "EVENTS_END must not be set before tick"
        );
        assert!(spi.needs_bus_tick(), "pending_start must be set");

        // Run EasyDMA.
        spi.tick_with_bus(&mut bus);

        // Completion events.
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            1,
            "EVENTS_END must be 1 after transfer"
        );
        assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX must be 1");
        assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX must be 1");

        // AMOUNT registers.
        assert_eq!(nrf_read_u32(&spi, 0x54C), 4, "TXD.AMOUNT must be 4");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 4, "RXD.AMOUNT must be 4");

        // No device/loopback → MISO is all zeros.
        let rx = bus.read_slice(rx_base, 4);
        assert_eq!(rx, vec![0, 0, 0, 0], "RXD RAM must be zeros with no device");

        // needs_bus_tick must be clear after completion.
        assert!(
            !spi.needs_bus_tick(),
            "pending_start must be cleared after tick_with_bus"
        );
    }

    /// Full EasyDMA transfer with loopback (MOSI → MISO mirror):
    /// RXD RAM should contain the same bytes that were transmitted.
    #[test]
    fn nrf52_spim_easydma_loopback_rxd_mirrors_txd() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.set_loopback(true);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0200;
        let rx_base: u64 = 0x2000_0300;
        let tx_data: [u8; 5] = [0x11, 0x22, 0x33, 0x44, 0x55];
        bus.write_slice(tx_base, &tx_data);

        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 5);
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 5);

        nrf_write_u32(&mut spi, 0x010, 1); // TASKS_START
        spi.tick_with_bus(&mut bus);

        // With loopback, each MISO byte is the same as the MOSI byte.
        let rx = bus.read_slice(rx_base, 5);
        assert_eq!(rx, tx_data.to_vec(), "loopback: RXD == TXD");
        assert_eq!(nrf_read_u32(&spi, 0x54C), 5, "TXD.AMOUNT");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 5, "RXD.AMOUNT");
        assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END");
    }

    /// Attached SpiDevice (echo slave): every MOSI byte is returned as-is.
    /// RXD RAM should contain the transmitted bytes.
    #[test]
    fn nrf52_spim_easydma_echo_device_rxd_contains_mosi() {
        struct EchoSlave;
        impl SpiDevice for EchoSlave {
            fn transfer(&mut self, mosi: u8) -> u8 {
                mosi
            }
            fn cs_pin(&self) -> &str {
                ""
            }
        }

        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.push_device(Box::new(EchoSlave));
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0400;
        let rx_base: u64 = 0x2000_0500;
        let tx_data: [u8; 3] = [0xA1, 0xB2, 0xC3];
        bus.write_slice(tx_base, &tx_data);

        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 3);
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 3);
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        let rx = bus.read_slice(rx_base, 3);
        assert_eq!(
            rx,
            tx_data.to_vec(),
            "echo device: RXD == TXD (MISO mirrors MOSI)"
        );
        assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END");
        assert_eq!(nrf_read_u32(&spi, 0x54C), 3, "TXD.AMOUNT == 3");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 3, "RXD.AMOUNT == 3");
    }

    #[test]
    fn nrf52_spim_gpio_cs_selects_only_matching_device_and_spans_transfers() {
        use std::sync::{Arc, Mutex};

        struct TransactionSlave(&'static str, Arc<Mutex<Vec<String>>>);
        impl SpiDevice for TransactionSlave {
            fn transfer(&mut self, _mosi: u8) -> u8 {
                0
            }
            fn cs_pin(&self) -> &str {
                "P0.12"
            }
            fn cs_select(&mut self) {
                self.1.lock().unwrap().push(format!("{}:select", self.0));
            }
            fn cs_release(&mut self) {
                self.1.lock().unwrap().push(format!("{}:release", self.0));
            }
        }

        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.push_device(Box::new(TransactionSlave("a", events.clone())));
        struct Other(TransactionSlave);
        impl SpiDevice for Other {
            fn transfer(&mut self, mosi: u8) -> u8 {
                self.0.transfer(mosi)
            }
            fn cs_pin(&self) -> &str {
                "P0.13"
            }
            fn cs_select(&mut self) {
                self.0.cs_select()
            }
            fn cs_release(&mut self) {
                self.0.cs_release()
            }
        }
        spi.push_device(Box::new(Other(TransactionSlave("b", events.clone()))));
        let mut bus = FlatRamBus::new();
        bus.gpio.insert("P0.12".into(), false);
        bus.gpio.insert("P0.13".into(), true);
        bus.write_slice(0x2000_0200, &[0xC0]);
        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x544, 0x2000_0200);
        nrf_write_u32(&mut spi, 0x548, 1);
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);
        assert_eq!(*events.lock().unwrap(), ["a:select"]);
        bus.gpio.insert("P0.12".into(), true);
        spi.tick_with_bus(&mut bus);
        assert_eq!(*events.lock().unwrap(), ["a:select", "a:release"]);
    }

    #[test]
    fn nrf52_spim_start_and_zero_length_do_not_invent_cs_pulse() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        struct Slave(std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
        impl SpiDevice for Slave {
            fn transfer(&mut self, _mosi: u8) -> u8 {
                0
            }
            fn cs_pin(&self) -> &str {
                "P0.12"
            }
            fn cs_select(&mut self) {
                self.0.lock().unwrap().push("select")
            }
            fn cs_release(&mut self) {
                self.0.lock().unwrap().push("release")
            }
        }
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.push_device(Box::new(Slave(events.clone())));
        let mut bus = FlatRamBus::new();
        bus.gpio.insert("P0.12".into(), true);
        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x548, 0);
        nrf_write_u32(&mut spi, 0x538, 0);
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);
        assert!(events.lock().unwrap().is_empty());
    }

    /// RXD.MAXCNT < TXD.MAXCNT: RXD fills up, remaining MISO bytes are discarded.
    /// TXD.AMOUNT == TXD.MAXCNT, RXD.AMOUNT == RXD.MAXCNT.
    #[test]
    fn nrf52_spim_easydma_rxd_maxcnt_limits_rxd_amount() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.set_loopback(true);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0600;
        let rx_base: u64 = 0x2000_0700;
        bus.write_slice(tx_base, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);

        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 6); // TXD.MAXCNT = 6
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 3); // RXD.MAXCNT = 3 (less)
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        assert_eq!(nrf_read_u32(&spi, 0x54C), 6, "TXD.AMOUNT == 6");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 3, "RXD.AMOUNT == 3 (clamped)");
        // Only first 3 bytes written to RX buffer.
        let rx = bus.read_slice(rx_base, 3);
        assert_eq!(rx, vec![0x01, 0x02, 0x03], "first 3 bytes received");
    }

    /// ORC (over-read character): when TXD.MAXCNT < RXD.MAXCNT, the ORC byte
    /// is clocked out for the extra cycles. With loopback, those ORC bytes
    /// end up in the RXD buffer.
    #[test]
    fn nrf52_spim_easydma_orc_pads_extra_rx_cycles() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.set_loopback(true);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0800;
        let rx_base: u64 = 0x2000_0900;
        bus.write_slice(tx_base, &[0xAA, 0xBB]); // 2 TX bytes

        nrf_write_u32(&mut spi, 0x5C0, 0xFF); // ORC = 0xFF
        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 2); // TXD.MAXCNT = 2
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 4); // RXD.MAXCNT = 4 (2 extra)
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        // TXD.AMOUNT counts actual TX bytes, not ORC clocks.
        assert_eq!(nrf_read_u32(&spi, 0x54C), 2, "TXD.AMOUNT == 2 (not 4)");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 4, "RXD.AMOUNT == 4");
        let rx = bus.read_slice(rx_base, 4);
        // Loopback: first 2 = TXD bytes, last 2 = ORC (0xFF).
        assert_eq!(rx, vec![0xAA, 0xBB, 0xFF, 0xFF], "ORC fills extra RX slots");
    }

    /// EVENTS write semantics: SW writing 1 to an EVENTS register must NOT set it.
    /// Only SW writing 0 clears it.
    #[test]
    fn nrf52_spim_events_write_1_ignored() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        let mut bus = FlatRamBus::new();

        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x548, 2);
        nrf_write_u32(&mut spi, 0x544, 0x2000_0000_u32);
        nrf_write_u32(&mut spi, 0x538, 2);
        nrf_write_u32(&mut spi, 0x534, 0x2000_0100_u32);

        // Arm and run transfer.
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);
        assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END set by HW");
        assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX set by HW");
        assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX set by HW");

        // SW write of 1 must be ignored (silicon-verified rule).
        nrf_write_u32(&mut spi, 0x118, 1); // attempt to SET EVENTS_END — must be ignored
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            1,
            "EVENTS_END unchanged by SW write of 1"
        );

        // SW write of 0 clears it.
        nrf_write_u32(&mut spi, 0x118, 0);
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            0,
            "EVENTS_END cleared by SW write of 0"
        );
        nrf_write_u32(&mut spi, 0x120, 0);
        assert_eq!(
            nrf_read_u32(&spi, 0x120),
            0,
            "EVENTS_ENDTX cleared by SW write of 0"
        );
        nrf_write_u32(&mut spi, 0x110, 0);
        assert_eq!(
            nrf_read_u32(&spi, 0x110),
            0,
            "EVENTS_ENDRX cleared by SW write of 0"
        );
    }

    /// TASKS_START before tick_with_bus: EVENTS must not be set immediately.
    /// They should only appear after tick_with_bus runs.
    #[test]
    fn nrf52_spim_events_not_set_before_tick_with_bus() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);

        nrf_write_u32(&mut spi, 0x500, 7);
        nrf_write_u32(&mut spi, 0x548, 1);
        nrf_write_u32(&mut spi, 0x544, 0x2000_0000_u32);

        // Before TASKS_START: no events.
        assert_eq!(nrf_read_u32(&spi, 0x118), 0, "EVENTS_END initially 0");
        assert_eq!(nrf_read_u32(&spi, 0x120), 0, "EVENTS_ENDTX initially 0");
        assert_eq!(nrf_read_u32(&spi, 0x110), 0, "EVENTS_ENDRX initially 0");

        // After TASKS_START but BEFORE tick_with_bus: still 0.
        nrf_write_u32(&mut spi, 0x010, 1);
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            0,
            "EVENTS_END must not fire before tick"
        );
        assert_eq!(nrf_read_u32(&spi, 0x120), 0, "EVENTS_ENDTX before tick");
        assert_eq!(nrf_read_u32(&spi, 0x110), 0, "EVENTS_ENDRX before tick");
    }

    /// INTENSET / INTENCLR round-trip.
    #[test]
    fn nrf52_spim_intenset_intenclr_round_trip() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);

        // INTENSET: bit 6 = INTEN_END, bit 8 = INTEN_ENDTX.
        nrf_write_u32(&mut spi, 0x304, (1 << 6) | (1 << 8));
        assert_eq!(
            nrf_read_u32(&spi, 0x304),
            (1 << 6) | (1 << 8),
            "INTENSET sets bits"
        );

        // INTENCLR: clear bit 6 only.
        nrf_write_u32(&mut spi, 0x308, 1 << 6);
        assert_eq!(nrf_read_u32(&spi, 0x308), 1 << 8, "INTENCLR clears bit 6");
    }

    /// ORC register stores only the low 8 bits.
    #[test]
    fn nrf52_spim_orc_masks_to_8_bits() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        nrf_write_u32(&mut spi, 0x5C0, 0xFFFF_FFAB);
        assert_eq!(
            nrf_read_u32(&spi, 0x5C0),
            0xAB,
            "ORC retains only low 8 bits"
        );
    }

    /// ENABLE register round-trip.
    #[test]
    fn nrf52_spim_enable_round_trip() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        nrf_write_u32(&mut spi, 0x500, 7);
        assert_eq!(nrf_read_u32(&spi, 0x500), 7, "ENABLE round-trips");
    }

    /// TASKS registers read back as 0 (write-only strobes on silicon).
    #[test]
    fn nrf52_spim_tasks_read_as_zero() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        nrf_write_u32(&mut spi, 0x010, 1); // TASKS_START
        assert_eq!(nrf_read_u32(&spi, 0x010), 0, "TASKS_START reads as 0");
    }

    /// Second TASKS_START after a completed transfer re-arms the engine.
    #[test]
    fn nrf52_spim_easydma_second_start_reruns_transfer() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        spi.set_loopback(true);
        let mut bus = FlatRamBus::new();

        let tx_base: u64 = 0x2000_0A00;
        let rx_base: u64 = 0x2000_0B00;
        bus.write_slice(tx_base, &[0x01, 0x02]);

        nrf_write_u32(&mut spi, 0x544, tx_base as u32);
        nrf_write_u32(&mut spi, 0x548, 2);
        nrf_write_u32(&mut spi, 0x534, rx_base as u32);
        nrf_write_u32(&mut spi, 0x538, 2);
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);
        assert_eq!(nrf_read_u32(&spi, 0x54C), 2);

        // Update TX buffer and run a second transfer.
        bus.write_slice(tx_base, &[0x55, 0x66]);
        nrf_write_u32(&mut spi, 0x118, 0); // clear EVENTS_END
        nrf_write_u32(&mut spi, 0x120, 0); // clear EVENTS_ENDTX
        nrf_write_u32(&mut spi, 0x110, 0); // clear EVENTS_ENDRX
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        let rx = bus.read_slice(rx_base, 2);
        assert_eq!(rx, vec![0x55, 0x66], "second transfer sees new TX data");
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            1,
            "EVENTS_END after second transfer"
        );
    }

    /// tick_with_bus with TXD.MAXCNT == 0 and RXD.MAXCNT == 0: completes
    /// immediately with AMOUNT == 0 and all events fired.
    #[test]
    fn nrf52_spim_easydma_zero_length_transfer() {
        let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
        let mut bus = FlatRamBus::new();

        nrf_write_u32(&mut spi, 0x544, 0x2000_0000);
        nrf_write_u32(&mut spi, 0x548, 0); // TXD.MAXCNT = 0
        nrf_write_u32(&mut spi, 0x534, 0x2000_0100);
        nrf_write_u32(&mut spi, 0x538, 0); // RXD.MAXCNT = 0
        nrf_write_u32(&mut spi, 0x010, 1);
        spi.tick_with_bus(&mut bus);

        assert_eq!(nrf_read_u32(&spi, 0x54C), 0, "TXD.AMOUNT == 0");
        assert_eq!(nrf_read_u32(&spi, 0x53C), 0, "RXD.AMOUNT == 0");
        assert_eq!(
            nrf_read_u32(&spi, 0x118),
            1,
            "EVENTS_END fires even for zero-length"
        );
        assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX fires");
        assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX fires");
    }

    // ── STM32H5 ("SPI v3", RM0481) unit tests ────────────────────────────────
    // Register-level expectations pinned by silicon capture 2026-06-11
    // (NUCLEO-H563ZI), probed over SWD. The TX data engine is spec-derived
    // (the bench part had no SPI kernel clock — see Stm32H5SpiRegs docs).

    fn h5() -> Spi {
        Spi::new_with_layout(SpiRegisterLayout::Stm32H5)
    }

    fn h5_read(spi: &Spi, offset: u64) -> u32 {
        spi.read_u32(offset).unwrap()
    }

    fn h5_write(spi: &mut Spi, offset: u64, value: u32) {
        spi.write_u32(offset, value).unwrap();
    }

    /// Master-mode bring-up: CR1.SSI=1, then CFG2 = MASTER|SSM, CR2.TSIZE.
    fn h5_master(tsize: u32) -> Spi {
        let mut spi = h5();
        h5_write(&mut spi, 0x00, 1 << 12); // CR1.SSI = 1 (internal SS high)
        h5_write(&mut spi, 0x0C, (1 << 22) | (1 << 26)); // CFG2 = MASTER|SSM
        h5_write(&mut spi, 0x04, tsize); // CR2.TSIZE
        spi
    }

    /// The chip-yaml token "stm32h5" selects the v3 layout, NOT the L4/F7
    /// FIFO map it used to alias.
    #[test]
    fn stm32h5_from_str_selects_v3_layout() {
        assert_eq!(
            "stm32h5".parse::<SpiRegisterLayout>().unwrap(),
            SpiRegisterLayout::Stm32H5
        );
        assert_eq!(
            "stm32l4".parse::<SpiRegisterLayout>().unwrap(),
            SpiRegisterLayout::Stm32Fifo,
            "L4/F7/G4 stay on the FIFO layout"
        );
    }

    /// Reset values — silicon capture 2026-06-11 (NUCLEO-H563ZI).
    #[test]
    fn stm32h5_reset_values_match_silicon() {
        let spi = h5();
        assert_eq!(h5_read(&spi, 0x00), 0, "CR1");
        assert_eq!(h5_read(&spi, 0x04), 0, "CR2");
        assert_eq!(h5_read(&spi, 0x08), 0x0007_0007, "CFG1");
        assert_eq!(h5_read(&spi, 0x0C), 0, "CFG2");
        assert_eq!(h5_read(&spi, 0x10), 0, "IER");
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "SR = TXP|TXC");
        assert_eq!(h5_read(&spi, 0x18), 0, "IFCR is write-only, reads 0");
        assert_eq!(h5_read(&spi, 0x20), 0, "TXDR is write-only, reads 0");
        assert_eq!(h5_read(&spi, 0x30), 0, "RXDR");
        assert_eq!(h5_read(&spi, 0x40), 0x0000_0107, "CRCPOLY");
        assert_eq!(h5_read(&spi, 0x44), 0, "TXCRC");
        assert_eq!(h5_read(&spi, 0x48), 0, "RXCRC");
        assert_eq!(h5_read(&spi, 0x4C), 0, "UDRDR");
        assert_eq!(h5_read(&spi, 0x50), 0, "I2SCFGR");
    }

    /// CFG1 writable mask — all three silicon round-trip pairs.
    #[test]
    fn stm32h5_cfg1_reserved_bits_masked() {
        let mut spi = h5();
        h5_write(&mut spi, 0x08, 0x7000_0007);
        assert_eq!(h5_read(&spi, 0x08), 0x7000_0007);
        h5_write(&mut spi, 0x08, 0x0008_0008);
        assert_eq!(h5_read(&spi, 0x08), 0x0008_0008);
        h5_write(&mut spi, 0x08, 0x5555_AAAA);
        assert_eq!(
            h5_read(&spi, 0x08),
            0x5055_82AA,
            "reserved bits 0x05002800 read as 0"
        );
    }

    /// CR2.TSIZE, CRCPOLY and IER round-trip the silicon-probed values.
    #[test]
    fn stm32h5_config_round_trips() {
        let mut spi = h5();
        h5_write(&mut spi, 0x04, 0x10);
        assert_eq!(h5_read(&spi, 0x04), 0x10, "CR2.TSIZE");
        h5_write(&mut spi, 0x40, 0xA5A5);
        assert_eq!(h5_read(&spi, 0x40), 0xA5A5, "CRCPOLY");
        h5_write(&mut spi, 0x10, 0x209);
        assert_eq!(h5_read(&spi, 0x10), 0x209, "IER");
    }

    /// MASTER is accepted when the internal SS level is high (SSM=1, SSI=1).
    #[test]
    fn stm32h5_cfg2_master_accepted_when_ssi_high() {
        let mut spi = h5();
        h5_write(&mut spi, 0x00, 1 << 12); // CR1.SSI = 1 first
        h5_write(&mut spi, 0x0C, (1 << 22) | (1 << 26));
        assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000);
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "no MODF");
    }

    /// Mode fault: MASTER requested with SSM=1 while SSI=0 → MASTER refused,
    /// SR.MODF latches, SPE is refused until IFCR clears MODF.
    #[test]
    fn stm32h5_mode_fault_refuses_master_and_blocks_spe() {
        let mut spi = h5();
        // SSI is 0 at reset: the MASTER|SSM request mode-faults.
        h5_write(&mut spi, 0x0C, 0x0440_0000);
        assert_eq!(h5_read(&spi, 0x0C), 0x0400_0000, "MASTER stored as 0");
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1202, "SR = TXP|MODF|TXC");
        // SPE refused while the fault stands.
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1000, "SPE refused, SSI kept");
        // IFCR bit 9 clears MODF; MASTER and SPE then go through.
        h5_write(&mut spi, 0x18, 1 << 9);
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "MODF cleared via IFCR");
        h5_write(&mut spi, 0x0C, 0x0440_0000);
        assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000, "MASTER accepted (SSI=1)");
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12));
        assert_eq!(h5_read(&spi, 0x00) & 1, 1, "SPE accepted after clear");
    }

    /// While SPE=1 the configuration registers are locked: CFG1/CFG2 writes
    /// are ignored.
    #[test]
    fn stm32h5_spe_locks_cfg1_and_cfg2() {
        let mut spi = h5_master(2);
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
        h5_write(&mut spi, 0x0C, 0x0440_0000 | (1 << 29));
        assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000, "CFG2 locked under SPE");
        h5_write(&mut spi, 0x08, 0x7000_0007);
        assert_eq!(h5_read(&spi, 0x08), 0x0007_0007, "CFG1 locked under SPE");
    }

    /// Setting SPE loads SR.CTSIZE from CR2.TSIZE and clears TXC (a transfer
    /// is pending).
    #[test]
    fn stm32h5_spe_loads_ctsize_and_clears_txc() {
        let mut spi = h5_master(2);
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
        assert_eq!(h5_read(&spi, 0x14), 0x0002_0002, "CTSIZE=2, TXP, TXC off");
    }

    /// CR1.CSTART latches while a transfer is active and cannot be cleared by
    /// software (HW clears it at EOT — RM0481 §41.4.10).
    #[test]
    fn stm32h5_cstart_latches_while_transfer_active() {
        let mut spi = h5_master(2);
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12)); // SPE|CSTART|SSI
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART latched");
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // try to drop CSTART
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART not SW-clearable");
    }

    /// The bench TXDR/IFCR/SPE-clear sequence. CSTART is left clear so no
    /// frame shifts and CTSIZE stays put — exactly the unclocked-silicon
    /// behaviour captured on the bench.
    #[test]
    fn stm32h5_txdr_txtf_ifcr_and_spe_clear_sequence() {
        let mut spi = h5_master(2);
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
        h5_write(&mut spi, 0x20, 0xAB); // TXDR
        assert_eq!(h5_read(&spi, 0x14), 0x0002_0012, "TXP|TXTF, CTSIZE=2");
        h5_write(&mut spi, 0x18, 0xFFFF_FFFF); // IFCR: clear all clearables
        assert_eq!(h5_read(&spi, 0x14), 0x0002_0002, "TXTF cleared");
        h5_write(&mut spi, 0x00, 1 << 12); // SPE → 0
        assert_eq!(h5_read(&spi, 0x14), 0x0002_1002, "TXC set, CTSIZE kept");
    }

    /// Sim-side TX engine: with SPE+CSTART in master mode each TXDR write
    /// transmits one frame and decrements CTSIZE; at 0 → EOT|TXC, CSTART
    /// HW-cleared. RXDR stays 0 (TX-only model).
    #[test]
    fn stm32h5_tx_engine_transmits_and_completes() {
        let mut spi = h5_master(2);
        spi.push_device(Box::new(Capture { rx: Vec::new() }));
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12)); // SPE|CSTART|SSI
        h5_write(&mut spi, 0x20, 0x11);
        assert_eq!(
            h5_read(&spi, 0x14),
            0x0001_0013,
            "CTSIZE 2->1, TXP|TXTF|RXP (each frame clocks one in)"
        );
        h5_write(&mut spi, 0x20, 0x22);
        assert_eq!(captured(&spi), vec![0x11, 0x22], "both frames on the bus");
        assert_eq!(h5_read(&spi, 0x14), 0x0000_101B, "EOT|TXC|RXP at CTSIZE=0");
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1001, "CSTART HW-cleared");
        // Full duplex: each transmitted frame clocks one in. With no slave
        // attached the captured value is the idle line level (0), but RXP is
        // set and RXDR is readable — the receive EVENT happens regardless.
        // This previously asserted a TX-only engine, which hung every driver
        // that writes TXDR then waits on RXP (HAL_SPI_TransmitReceive, and so
        // Arduino SPI.transfer()).
        assert_eq!(h5_read(&spi, 0x30), 0, "RXDR holds the idle MISO level");
    }

    /// TXDR writes are inert while SPE=0: no TXTF, nothing transmitted.
    #[test]
    fn stm32h5_txdr_ignored_when_disabled() {
        let mut spi = h5_master(2);
        spi.push_device(Box::new(Capture { rx: Vec::new() }));
        h5_write(&mut spi, 0x20, 0xAB);
        assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "SR untouched");
        assert!(captured(&spi).is_empty(), "nothing transmitted");
    }

    /// TXDR byte/halfword accesses are each ONE frame (RM0481 §41.4.13:
    /// access size = frame size). TSIZE=0 = endless mode: CTSIZE stays 0,
    /// no EOT, CSTART stays latched.
    #[test]
    fn stm32h5_byte_and_halfword_txdr_access_is_one_frame() {
        let mut spi = h5_master(0); // TSIZE=0: endless
        spi.push_device(Box::new(Capture { rx: Vec::new() }));
        h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12));
        spi.write(0x20, 0x5A).unwrap(); // byte access → one 8-bit frame
        spi.write_u16(0x20, 0x1234).unwrap(); // halfword access → one frame
        assert_eq!(captured(&spi), vec![0x5A, 0x34], "low byte per frame");
        assert_eq!(h5_read(&spi, 0x14) >> 16, 0, "CTSIZE stays 0");
        assert_eq!(h5_read(&spi, 0x14) & (1 << 3), 0, "no EOT in endless mode");
        assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART stays latched");
    }

    /// Config registers are 32-bit with byte-merge semantics on the byte path.
    #[test]
    fn stm32h5_byte_writes_merge_into_32bit_registers() {
        let mut spi = h5();
        spi.write(0x40, 0xA5).unwrap(); // CRCPOLY low byte (reset 0x107)
        spi.write(0x41, 0x5A).unwrap(); // CRCPOLY byte 1
        assert_eq!(h5_read(&spi, 0x40), 0x0000_5AA5, "bytes merged in place");
    }
}
