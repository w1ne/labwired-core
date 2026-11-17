// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SPI on GPIO pins: a slave-side bus decoder that puts any [`SpiDevice`] on
//! arbitrary MCU pads.
//!
//! A device declared with `connection: spi_gpio` is not attached to an SPI
//! controller. The decoder watches the pads the manifest names, exactly as a
//! slave on the board would:
//!
//! ```yaml
//! external_devices:
//!   - id: lcd
//!     type: st7920
//!     connection: spi_gpio
//!     config:
//!       sck: PA1
//!       mosi: PA2
//!       miso: PA3        # optional: read-back pad
//!       cs: PA0          # optional: absent = always selected
//!       cs_active: high  # high | low; default: the device's own polarity
//!       mode: 0          # 0..3 (bit 1 = CPOL, bit 0 = CPHA); default 0
//!       bit_order: msb   # msb | lsb; default msb
//!       controller: spi  # optional: also reachable from that hardware SPI
//! ```
//!
//! Wire semantics follow the SPI mode definitions (CPOL = SCK idle level,
//! CPHA = 0 samples on the leading edge and shifts on the trailing edge,
//! CPHA = 1 the other way round):
//!
//! * MOSI is sampled on the mode's sampling edge, at the level the pad carried
//!   before the store that made the edge (a store that moves MOSI and SCK
//!   together is a setup violation; the old bit wins, as in the controller
//!   bit engine, see `peripherals::spi::edge_slave_capture`).
//! * Eight samples make one byte, assembled MSB- or LSB-first, which goes to
//!   the device through the same [`SpiDevice::transfer`] a hardware
//!   controller uses.
//! * MISO carries the byte the device will answer
//!   ([`SpiDevice::next_miso`]): at CPHA = 0 its first bit is on the pad from
//!   the chip-select edge (and from the trailing edge that ends the previous
//!   byte), each following bit after a trailing edge; at CPHA = 1 each bit
//!   goes out on a leading edge.
//! * Chip select: the CS pad's active level selects the device; either CS
//!   edge drops a partial byte and calls `cs_select` / `cs_release`, which is
//!   what resets a device's transaction state on a hardware bus. SCK edges
//!   while deselected are ignored.
//!
//! The decoder reads the MCU's output LATCH of each pad, not the pad: a pad a
//! hardware SPI controller owns through its alternate function does not move
//! the latch, so hardware traffic is never decoded twice. That traffic reaches
//! the same device when `controller:` names the controller: the device is then
//! shared between the decoder and the controller (one model, one CS state), so
//! a firmware may drive it either way. Pads move only on firmware stores, so
//! the decoder runs from the bus write hook
//! ([`BusResidentDevice::edge_service_addrs`]) and costs nothing per tick.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::bus::{BusPayload, BusResidentDevice, DevicePins, SystemBus};
use crate::peripherals::kit::AttachCtx;
use crate::peripherals::spi::SpiDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// The `connection:` value that selects this decoder.
pub const CONNECTION: &str = "spi_gpio";

/// Frame shape of one decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpiGpioMode {
    /// SCK idle level.
    pub cpol: bool,
    /// Sample on the trailing (true) or leading (false) edge.
    pub cpha: bool,
    pub lsb_first: bool,
}

impl SpiGpioMode {
    /// SPI mode number: bit 1 = CPOL, bit 0 = CPHA.
    pub const fn from_mode(mode: u8, lsb_first: bool) -> Self {
        Self {
            cpol: mode & 0b10 != 0,
            cpha: mode & 0b01 != 0,
            lsb_first,
        }
    }

    /// Bit `idx` (in shift order) of `byte`.
    fn bit(self, byte: u8, idx: u8) -> bool {
        let pos = if self.lsb_first { idx } else { 7 - idx };
        (byte >> pos) & 1 != 0
    }

    fn set_bit(self, byte: &mut u8, idx: u8, level: bool) {
        let pos = if self.lsb_first { idx } else { 7 - idx };
        if level {
            *byte |= 1 << pos;
        }
    }
}

/// A resolved pad: `(register address, bit)`.
pub type Pad = (u64, u8);

/// Pads one decoder observes and drives. SCK, MOSI and CS are the MCU's
/// output-latch registers; MISO is the input register the firmware reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiGpioPads {
    pub sck: Pad,
    pub mosi: Pad,
    pub miso: Option<Pad>,
    pub cs: Option<Pad>,
}

/// One device model shared by the decoder and, optionally, a hardware SPI
/// controller. The selection flag lives here so both paths agree on it and a
/// CS edge seen by both selects or releases the device once.
pub(crate) struct SharedSpi {
    dev: Box<dyn SpiDevice>,
    selected: bool,
}

impl std::fmt::Debug for SharedSpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedSpi")
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl SharedSpi {
    fn set_selected(&mut self, on: bool) {
        if on == self.selected {
            return;
        }
        self.selected = on;
        if on {
            self.dev.cs_select();
        } else {
            self.dev.cs_release();
        }
    }
}

type Shared = Arc<Mutex<SharedSpi>>;

fn lock(shared: &Shared) -> MutexGuard<'_, SharedSpi> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// The slave-side SPI decoder on GPIO pads.
#[derive(Debug)]
pub struct SpiGpioDecoder {
    id: String,
    shared: Shared,
    mode: SpiGpioMode,
    cs_active_high: bool,
    pads: SpiGpioPads,
    addrs: Vec<u64>,
    /// Whether the inspect walk lists the device here. False when a hardware
    /// controller also holds it and reports it already.
    reports: bool,
    trace: Option<crate::bus::bus_trace::BusTrace>,
    // Wire state, as of the last store this decoder saw.
    sck: bool,
    mosi: bool,
    selected: bool,
    bits: u8,
    rx: u8,
    /// The byte being shifted out on MISO, once loaded for this frame.
    tx: Option<u8>,
    miso_level: Option<bool>,
}

impl SpiGpioDecoder {
    /// A decoder owning `dev`. Call [`Self::sync`] with the machine's pins
    /// before the first store so the current levels are not taken as edges.
    pub fn new(
        id: impl Into<String>,
        dev: Box<dyn SpiDevice>,
        pads: SpiGpioPads,
        mode: SpiGpioMode,
        cs_active_high: bool,
    ) -> Self {
        Self::with_shared(
            id.into(),
            Arc::new(Mutex::new(SharedSpi {
                dev,
                selected: false,
            })),
            pads,
            mode,
            cs_active_high,
        )
    }

    fn with_shared(
        id: String,
        shared: Shared,
        pads: SpiGpioPads,
        mode: SpiGpioMode,
        cs_active_high: bool,
    ) -> Self {
        let mut addrs: Vec<u64> = [Some(pads.sck), Some(pads.mosi), pads.cs]
            .into_iter()
            .flatten()
            .map(|(addr, _)| addr)
            .collect();
        addrs.sort_unstable();
        addrs.dedup();
        Self {
            id,
            shared,
            mode,
            cs_active_high,
            pads,
            addrs,
            reports: true,
            trace: None,
            sck: mode.cpol,
            mosi: false,
            selected: false,
            bits: 0,
            rx: 0,
            tx: None,
            miso_level: None,
        }
    }

    /// Run `f` on the device this decoder feeds.
    pub fn with_device<R>(&self, f: impl FnOnce(&dyn SpiDevice) -> R) -> R {
        f(lock(&self.shared).dev.as_ref())
    }

    /// Take the current pad levels as the starting state (no edges), and
    /// select the device if CS already reads active (or there is no CS).
    pub fn sync(&mut self, pins: &mut dyn DevicePins) {
        self.sck = Self::level(pins, self.pads.sck).unwrap_or(self.mode.cpol);
        self.mosi = Self::level(pins, self.pads.mosi).unwrap_or(false);
        let selected = self.cs_level(pins);
        self.apply_cs(selected, pins);
    }

    fn level(pins: &dyn DevicePins, (addr, bit): Pad) -> Option<bool> {
        pins.output_bit(addr, bit)
    }

    fn cs_level(&self, pins: &dyn DevicePins) -> bool {
        match self.pads.cs {
            None => true,
            Some(pad) => Self::level(pins, pad).is_some_and(|high| high == self.cs_active_high),
        }
    }

    fn drive_miso(&mut self, pins: &mut dyn DevicePins, level: bool) {
        let Some((addr, bit)) = self.pads.miso else {
            return;
        };
        if self.miso_level != Some(level) {
            self.miso_level = Some(level);
            pins.drive_input_bit(addr, bit, level);
        }
    }

    /// The byte to shift out this frame, loaded once per frame.
    fn load_tx(&mut self) -> u8 {
        if let Some(byte) = self.tx {
            return byte;
        }
        let byte = lock(&self.shared).dev.next_miso().unwrap_or(0xFF);
        self.tx = Some(byte);
        byte
    }

    /// Start or end a selection: drop any partial byte either way.
    fn apply_cs(&mut self, selected: bool, pins: &mut dyn DevicePins) {
        if selected != self.selected {
            self.bits = 0;
            self.rx = 0;
            self.tx = None;
            self.selected = selected;
        }
        lock(&self.shared).set_selected(selected);
        if !selected {
            // A deselected slave lets go of MISO; the line reads high
            // through the pull-up an SPI bus with a released MISO carries.
            self.drive_miso(pins, true);
        } else if !self.mode.cpha && self.bits == 0 && self.tx.is_none() {
            // CPHA = 0: the first bit is on MISO before the first edge.
            let byte = self.load_tx();
            self.drive_miso(pins, self.mode.bit(byte, 0));
        }
    }

    fn on_sck_edge(&mut self, sck: bool, mosi_before: bool, pins: &mut dyn DevicePins) {
        let leading = sck != self.mode.cpol;
        if leading != self.mode.cpha {
            // Sampling edge.
            self.load_tx();
            self.mode.set_bit(&mut self.rx, self.bits, mosi_before);
            self.bits += 1;
            if self.bits == 8 {
                let mosi = self.rx;
                let miso = lock(&self.shared).dev.transfer(mosi);
                if let Some(trace) = &self.trace {
                    trace.push(CONNECTION, BusPayload::Spi { mosi, miso });
                }
                self.bits = 0;
                self.rx = 0;
                self.tx = None;
            }
        } else {
            // Shift edge: the next bit, or the next byte's first bit.
            let byte = self.load_tx();
            let level = self.mode.bit(byte, self.bits);
            self.drive_miso(pins, level);
        }
    }
}

impl BusResidentDevice for SpiGpioDecoder {
    /// Nothing happens between stores: see [`Self::service_edge`].
    fn service(&mut self, _pins: &mut dyn DevicePins, _now: u64) {}

    fn service_edge(&mut self, pins: &mut dyn DevicePins, _now: u64) {
        let sck = Self::level(pins, self.pads.sck).unwrap_or(self.sck);
        let mosi = Self::level(pins, self.pads.mosi).unwrap_or(self.mosi);
        let mosi_before = self.mosi;
        let selected = self.cs_level(pins);
        if selected != self.selected {
            // A CS edge in the same store as an SCK edge: CS wins, the clock
            // level is simply the new idle reference.
            self.apply_cs(selected, pins);
        } else if selected && sck != self.sck {
            self.on_sck_edge(sck, mosi_before, pins);
        }
        self.sck = sck;
        self.mosi = mosi;
    }

    fn needs_per_cycle_service(&self) -> bool {
        false
    }

    fn edge_service_addrs(&self) -> &[u64] {
        &self.addrs
    }

    fn is_attached_device(&self) -> bool {
        self.reports
    }

    fn evidence(&self) -> Option<&dyn crate::inspect::DeviceEvidence> {
        Some(self)
    }

    fn as_sim_input(&mut self) -> &mut dyn SimInput {
        self
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl crate::inspect::DeviceEvidence for SpiGpioDecoder {
    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        lock(&self.shared).dev.artifacts(id, opts)
    }
}

impl SimInput for SpiGpioDecoder {
    fn input_channels(&self) -> &[InputChannel] {
        &[]
    }
    fn set_input(&mut self, key: &str, _value: f64) -> Result<(), SimInputError> {
        Err(SimInputError::UnknownChannel(key.to_owned()))
    }
    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

/// The shared device as a hardware controller sees it: CS pin and polarity
/// are the decoder's, selection is the shared flag.
struct ControllerHandle {
    shared: Shared,
    cs_pin: String,
    cs_active_high: bool,
    component_id: Option<String>,
}

impl SpiDevice for ControllerHandle {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }
    fn cs_active_high(&self) -> bool {
        self.cs_active_high
    }
    fn cs_select(&mut self) {
        lock(&self.shared).set_selected(true);
    }
    fn cs_release(&mut self) {
        lock(&self.shared).set_selected(false);
    }
    fn is_selected(&self) -> bool {
        lock(&self.shared).selected
    }
    fn transfer(&mut self, mosi: u8) -> u8 {
        lock(&self.shared).dev.transfer(mosi)
    }
    fn next_miso(&self) -> Option<u8> {
        lock(&self.shared).dev.next_miso()
    }
    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }
    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        lock(&self.shared).dev.artifacts(id, opts)
    }
    fn advance_time_us(&mut self, us: u64) {
        lock(&self.shared).dev.advance_time_us(us);
    }
}

fn pad_label<'a>(ctx: &'a AttachCtx<'_>, key: &str) -> Option<&'a str> {
    ctx.config_str(key).map(str::trim).filter(|s| !s.is_empty())
}

/// Attach `device` through a GPIO decoder, as the `spi_gpio` connection asks.
/// Called from [`AttachCtx::attach_spi_device`], so every SPI kit gets it.
pub(crate) fn attach(ctx: &mut AttachCtx<'_>, device: Box<dyn SpiDevice>) -> anyhow::Result<()> {
    let id = ctx.device_id().to_string();
    let output = |ctx: &AttachCtx<'_>, key: &str, required: bool| -> anyhow::Result<Option<Pad>> {
        let Some(label) = pad_label(ctx, key) else {
            anyhow::ensure!(
                !required,
                "{id}: spi_gpio needs config `{key}` (a pin label)"
            );
            return Ok(None);
        };
        ctx.resolve_pin_odr(label).map(Some).ok_or_else(|| {
            anyhow::anyhow!("{id}: spi_gpio `{key}` pin '{label}' is not an MCU GPIO output")
        })
    };
    let sck = output(ctx, "sck", true)?.expect("required");
    let mosi = output(ctx, "mosi", true)?.expect("required");
    let cs = output(ctx, "cs", false)?;
    let miso =
        match pad_label(ctx, "miso") {
            None => None,
            Some(label) => Some(SystemBus::resolve_pin_idr_pub(&*ctx.bus, label).ok_or_else(
                || anyhow::anyhow!("{id}: spi_gpio `miso` pin '{label}' is not an MCU GPIO input"),
            )?),
        };
    let mode = match ctx.ext.config.get("mode") {
        None => 0,
        Some(v) => match v
            .as_i64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        {
            Some(m @ 0..=3) => m as u8,
            _ => anyhow::bail!("{id}: spi_gpio `mode` must be 0, 1, 2 or 3, got {v:?}"),
        },
    };
    let lsb_first = match pad_label(ctx, "bit_order").map(str::to_ascii_lowercase) {
        None => false,
        Some(o) if o == "msb" => false,
        Some(o) if o == "lsb" => true,
        Some(o) => anyhow::bail!("{id}: spi_gpio `bit_order` must be msb or lsb, got '{o}'"),
    };
    let cs_active_high = match pad_label(ctx, "cs_active").map(str::to_ascii_lowercase) {
        None => device.cs_active_high(),
        Some(p) if p == "high" => true,
        Some(p) if p == "low" => false,
        Some(p) => anyhow::bail!("{id}: spi_gpio `cs_active` must be high or low, got '{p}'"),
    };
    anyhow::ensure!(
        miso.is_none() || device.next_miso().is_some(),
        "{id}: spi_gpio `miso` needs a device that knows its next MISO byte before the frame \
         (SpiDevice::next_miso); type '{}' does not, so its read-back cannot be put on a pad",
        ctx.device_type()
    );
    let component_id = device.component_id().map(str::to_owned);
    let pads = SpiGpioPads {
        sck,
        mosi,
        miso,
        cs,
    };
    let mut decoder = SpiGpioDecoder::new(
        id.clone(),
        device,
        pads,
        SpiGpioMode::from_mode(mode, lsb_first),
        cs_active_high,
    );
    decoder.trace = Some(ctx.bus.bus_trace.clone());
    if let Some(controller) = pad_label(ctx, "controller").map(str::to_owned) {
        let handle = ControllerHandle {
            shared: decoder.shared.clone(),
            cs_pin: pad_label(ctx, "cs").unwrap_or("").to_string(),
            cs_active_high,
            component_id,
        };
        ctx.bus
            .attach_spi_device(&controller, Box::new(handle))
            .map_err(|e| anyhow::anyhow!("{id}: spi_gpio `controller` '{controller}': {e:#}"))?;
        decoder.reports = false;
    }
    decoder.sync(&mut *ctx.bus);
    ctx.bus.gpio_devices.push(Box::new(decoder));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, VecDeque};

    // Pads on one fake port; MISO is read back from a separate input register.
    const OUT: u64 = 0x100;
    const IN: u64 = 0x200;
    const SCK: Pad = (OUT, 1);
    const MOSI: Pad = (OUT, 2);
    const CS: Pad = (OUT, 0);
    const MISO: Pad = (IN, 3);

    #[derive(Default)]
    struct Pins {
        out: BTreeMap<Pad, bool>,
        input: BTreeMap<Pad, bool>,
    }
    impl DevicePins for Pins {
        fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            Some(self.out.get(&(addr, bit)).copied().unwrap_or(false))
        }
        fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
            self.input.insert((addr, bit), high);
        }
        fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool {
            self.input.insert((addr, bit), high);
            true
        }
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    struct Log {
        events: Vec<String>,
        received: Vec<u8>,
    }

    /// A slave that records what it is sent and answers from a queue, its
    /// answer for a frame fixed before the frame (as a shift register is).
    struct Probe {
        log: Arc<Mutex<Log>>,
        answers: VecDeque<u8>,
        cs_active_high: bool,
    }
    impl SpiDevice for Probe {
        fn cs_pin(&self) -> &str {
            ""
        }
        fn cs_active_high(&self) -> bool {
            self.cs_active_high
        }
        fn cs_select(&mut self) {
            self.log.lock().unwrap().events.push("select".into());
        }
        fn cs_release(&mut self) {
            self.log.lock().unwrap().events.push("release".into());
        }
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.log.lock().unwrap().received.push(mosi);
            self.answers.pop_front().unwrap_or(0xFF)
        }
        fn next_miso(&self) -> Option<u8> {
            Some(self.answers.front().copied().unwrap_or(0xFF))
        }
    }

    /// A master as the SPI mode definitions draw it, driving the fake port
    /// one store at a time and servicing the decoder after each store.
    struct Rig {
        pins: Pins,
        dec: SpiGpioDecoder,
        log: Arc<Mutex<Log>>,
        master: SpiGpioMode,
        cs_active_high: bool,
    }

    impl Rig {
        fn new(slave: SpiGpioMode, master: SpiGpioMode, answers: &[u8]) -> Self {
            Self::with_cs(slave, master, answers, false)
        }

        fn with_cs(
            slave: SpiGpioMode,
            master: SpiGpioMode,
            answers: &[u8],
            cs_active_high: bool,
        ) -> Self {
            let log = Arc::new(Mutex::new(Log::default()));
            let probe = Probe {
                log: log.clone(),
                answers: answers.iter().copied().collect(),
                cs_active_high,
            };
            let mut pins = Pins::default();
            pins.out.insert(SCK, master.cpol);
            pins.out.insert(CS, !cs_active_high);
            let mut dec = SpiGpioDecoder::new(
                "probe",
                Box::new(probe),
                SpiGpioPads {
                    sck: SCK,
                    mosi: MOSI,
                    miso: Some(MISO),
                    cs: Some(CS),
                },
                slave,
                cs_active_high,
            );
            dec.sync(&mut pins);
            Self {
                pins,
                dec,
                log,
                master,
                cs_active_high,
            }
        }

        fn store(&mut self, pad: Pad, level: bool) {
            self.pins.out.insert(pad, level);
            self.dec.service_edge(&mut self.pins, 0);
        }

        fn miso(&self) -> bool {
            self.pins.input.get(&MISO).copied().unwrap_or(false)
        }

        fn cs(&mut self, active: bool) {
            self.store(CS, active == self.cs_active_high);
        }

        /// Clock `n` bits of `byte` out (in the master's bit order) and
        /// return what the master sampled on MISO, as the mode definitions
        /// say: CPHA = 0 puts the bit out before the leading edge and samples
        /// on it; CPHA = 1 puts the bit out on the leading edge and samples
        /// on the trailing edge.
        fn clock_bits(&mut self, byte: u8, n: u8) -> u8 {
            let m = self.master;
            let mut got = 0u8;
            for i in 0..n {
                let bit = m.bit(byte, i);
                if !m.cpha {
                    self.store(MOSI, bit);
                    self.store(SCK, !m.cpol); // leading: sample
                    let level = self.miso();
                    m.set_bit(&mut got, i, level);
                    self.store(SCK, m.cpol); // trailing: shift
                } else {
                    self.store(SCK, !m.cpol); // leading: shift
                    self.store(MOSI, bit);
                    self.store(SCK, m.cpol); // trailing: sample
                    let level = self.miso();
                    m.set_bit(&mut got, i, level);
                }
            }
            got
        }

        fn xfer(&mut self, byte: u8) -> u8 {
            self.clock_bits(byte, 8)
        }

        fn received(&self) -> Vec<u8> {
            self.log.lock().unwrap().received.clone()
        }

        fn events(&self) -> Vec<String> {
            self.log.lock().unwrap().events.clone()
        }
    }

    fn mode(n: u8) -> SpiGpioMode {
        SpiGpioMode::from_mode(n, false)
    }

    /// Modes 0-3, matched master and slave: the byte goes in and the slave's
    /// queued answer comes back intact, full duplex, byte after byte.
    #[test]
    fn every_mode_exchanges_full_duplex_bytes() {
        for n in 0..4 {
            let mut rig = Rig::new(mode(n), mode(n), &[0x3C, 0x81]);
            rig.cs(true);
            assert_eq!(rig.xfer(0xA5), 0x3C, "mode {n}: first answer");
            assert_eq!(rig.xfer(0x5A), 0x81, "mode {n}: second answer");
            assert_eq!(rig.xfer(0x00), 0xFF, "mode {n}: empty queue idles high");
            rig.cs(false);
            assert_eq!(rig.received(), vec![0xA5, 0x5A, 0x00], "mode {n}");
        }
    }

    /// CPHA = 0: the slave's first bit must be on MISO when CS asserts,
    /// before any clock edge (the leading edge is already a sampling edge).
    #[test]
    fn cpha0_presents_the_first_bit_at_cs_assert() {
        for n in [0, 2] {
            let mut rig = Rig::new(mode(n), mode(n), &[0x80]);
            rig.store(MISO, false); // the decoder has not driven it yet
            rig.cs(true);
            assert!(rig.miso(), "mode {n}: MSB of 0x80 on MISO before SCK");
        }
    }

    /// Mode 0 samples on the rising edge: MOSI changed while SCK is high (after
    /// the sampling edge) is not what the slave latched.
    #[test]
    fn mode0_samples_rising_edge_not_falling() {
        let mut rig = Rig::new(mode(0), mode(0), &[]);
        rig.cs(true);
        for i in 0..8 {
            let bit = (0xC3u8 >> (7 - i)) & 1 != 0;
            rig.store(MOSI, bit);
            rig.store(SCK, true); // rising: sample
            rig.store(MOSI, !bit); // garbage while high
            rig.store(SCK, false); // falling
        }
        assert_eq!(rig.received(), vec![0xC3]);
    }

    /// Mode 1 samples on the falling edge: the level before the rising edge
    /// is not latched, the level before the falling edge is.
    #[test]
    fn mode1_samples_falling_edge_not_rising() {
        let mut rig = Rig::new(mode(1), mode(1), &[]);
        rig.cs(true);
        for i in 0..8 {
            let bit = (0xC3u8 >> (7 - i)) & 1 != 0;
            rig.store(MOSI, !bit); // garbage before the rising edge
            rig.store(SCK, true); // rising: shift
            rig.store(MOSI, bit);
            rig.store(SCK, false); // falling: sample
        }
        assert_eq!(rig.received(), vec![0xC3]);
    }

    /// CPOL = 1 inverts the idle level: mode 2 samples on the falling edge
    /// (its leading edge), mode 3 on the rising edge (its trailing edge).
    /// MOSI holds the bit only across the sampling edge; around the other
    /// edge it carries the inverse.
    #[test]
    fn cpol1_moves_the_sampling_edge() {
        for n in [2u8, 3] {
            let mut rig = Rig::new(mode(n), mode(n), &[]);
            rig.cs(true);
            for i in 0..8 {
                let bit = (0x96u8 >> (7 - i)) & 1 != 0;
                if n == 2 {
                    rig.store(MOSI, bit);
                    rig.store(SCK, false); // falling: sample
                    rig.store(MOSI, !bit);
                    rig.store(SCK, true); // rising: shift
                } else {
                    rig.store(MOSI, !bit);
                    rig.store(SCK, false); // falling: shift
                    rig.store(MOSI, bit);
                    rig.store(SCK, true); // rising: sample
                }
            }
            assert_eq!(rig.received(), vec![0x96], "mode {n}");
        }
    }

    /// A CPHA mismatch is not invisible: a mode 1 master talking to a mode 0
    /// slave puts each bit out after the slave sampled, shifting the byte.
    #[test]
    fn cpha_mismatch_corrupts_the_byte() {
        let mut rig = Rig::new(mode(0), mode(1), &[]);
        rig.cs(true);
        rig.xfer(0xF0);
        assert_ne!(rig.received(), vec![0xF0]);
    }

    /// LSB-first: the first bit on the wire is bit 0.
    #[test]
    fn lsb_first_assembles_bit0_first() {
        let lsb = SpiGpioMode::from_mode(0, true);
        let mut rig = Rig::new(lsb, lsb, &[0x01]);
        rig.cs(true);
        assert_eq!(rig.xfer(0x01), 0x01);
        assert_eq!(rig.received(), vec![0x01]);
        // The same wire, read MSB-first, is the mirrored byte.
        let mut rig = Rig::new(mode(0), lsb, &[]);
        rig.cs(true);
        rig.xfer(0x01);
        assert_eq!(rig.received(), vec![0x80]);
    }

    /// A CS edge mid-byte drops the partial byte and restarts the device's
    /// transaction (release, then select), like a hardware CS.
    #[test]
    fn cs_edge_aborts_a_partial_byte() {
        let mut rig = Rig::new(mode(0), mode(0), &[]);
        rig.cs(true);
        rig.clock_bits(0xFF, 3);
        rig.cs(false);
        rig.cs(true);
        rig.xfer(0x42);
        rig.cs(false);
        assert_eq!(rig.received(), vec![0x42]);
        assert_eq!(
            rig.events(),
            vec!["select", "release", "select", "release"],
            "each CS edge reaches the device once"
        );
    }

    /// Clocks while CS is inactive are not the device's traffic.
    #[test]
    fn deselected_clocks_are_ignored() {
        let mut rig = Rig::new(mode(0), mode(0), &[]);
        rig.xfer(0x11);
        assert!(rig.received().is_empty());
        rig.cs(true);
        rig.xfer(0x22);
        assert_eq!(rig.received(), vec![0x22]);
    }

    /// CS polarity is per device: an active-high slave is selected by CS high
    /// and ignores traffic while CS is low.
    #[test]
    fn active_high_cs_selects_on_high() {
        let mut rig = Rig::with_cs(mode(0), mode(0), &[], true);
        assert!(rig.events().is_empty(), "CS low at start: not selected");
        rig.store(CS, false);
        rig.xfer(0x11);
        assert!(rig.received().is_empty(), "CS low is inactive");
        rig.store(CS, true);
        rig.xfer(0x22);
        assert_eq!(rig.received(), vec![0x22]);
        assert_eq!(rig.events(), vec!["select"]);
    }

    /// A deselected slave releases MISO, which reads high.
    #[test]
    fn deselect_releases_miso_high() {
        let mut rig = Rig::new(mode(0), mode(0), &[0x00]);
        rig.cs(true);
        assert!(!rig.miso(), "0x00 MSB on MISO");
        rig.cs(false);
        assert!(rig.miso());
    }

    /// One store that moves MOSI and the sampling edge together: the slave
    /// latches the level the line held before the edge.
    #[test]
    fn simultaneous_mosi_and_edge_latch_the_old_level() {
        let mut rig = Rig::new(mode(0), mode(0), &[]);
        rig.cs(true);
        for _ in 0..8 {
            rig.pins.out.insert(MOSI, true);
            rig.store(SCK, true); // MOSI and SCK in one store
            rig.pins.out.insert(MOSI, false);
            rig.store(SCK, false);
        }
        assert_eq!(rig.received(), vec![0x00]);
    }
}
