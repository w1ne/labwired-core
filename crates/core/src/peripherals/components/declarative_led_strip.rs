// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic **declarative LED strip** — one engine device that interprets a
//! [`labwired_config::LedStripSpec`], so an addressable strip is a YAML file
//! rather than two unrelated hand-written models.
//!
//! Why a primitive of its own, and not `display`
//! =============================================
//! An addressable strip is a PER-LED COLOUR ARRAY clocked by a wire protocol.
//! It has no address counter, no command table, no window, and no frame memory
//! a later command re-reads — the four things the `display` primitive exists to
//! interpret. Expressing a strip as a display would have meant inventing all
//! four and then explaining, in every descriptor, that none of them moves.
//!
//! What is data
//! ------------
//! The colour order on the wire, the default strip length, which protocol
//! clocks the LEDs in, the bit-timing table a single-wire datasheet states in
//! nanoseconds, the SPI frame layout a clocked datasheet states in bytes, and
//! the artifact's published `meta` keys.
//!
//! What is engine
//! --------------
//! This file: the two decoders and the artifact. One implementation of each,
//! shared by every strip on that wire.
//!
//! One struct, two wires
//! ---------------------
//! [`GenericLedStrip`] answers on BOTH doors, because the two parts differ in
//! how bytes arrive and in nothing else:
//!
//! * `wire: spi_frames` — a [`SpiDevice`]. Bytes accumulate while CS is low and
//!   the strip LATCHES on CS release, which is what silicon does: a transaction
//!   shorter than a start frame plus one LED frame leaves the previous colours
//!   untouched, so a glitchy transfer cannot blank a strip.
//! * `wire: nrz_gpio` — a [`GpioObserver`](crate::peripherals::device::GpioObserver)
//!   on ONE pad. Every bit is a HIGH pulse whose DURATION is the bit value, and
//!   a long LOW gap latches the frame. Nothing about the byte stream is
//!   inferred: the decode reads real edge times off the pad the RMT drives.
//!
//! The observer hook is `&self`, so the decode state of a `nrz_gpio` strip
//! lives behind a `Mutex` and the part is held as `Arc<GenericLedStrip>`.

use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use labwired_config::{DeviceDescriptor, LedStripMetaFlag, LedStripSpec, LedStripWire};

use crate::peripherals::spi::SpiDevice;

/// One latched LED: its colour bytes in the order the artifact publishes them
/// (which `artifact_format` names), plus the global-brightness field when the
/// wire carries one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedPixel {
    /// Colour bytes as the artifact publishes them. For a single-wire strip
    /// that is the wire order verbatim — a twin that re-ordered them could not
    /// be compared against a capture of the data line; for a clocked strip it
    /// is whatever `spi_frames.colour_bytes` selects out of one LED frame.
    pub wire: [u8; 3],
    /// Global brightness, or 0 on a wire that has no such field.
    pub brightness: u8,
}

/// Convert a nanosecond span to sim cycles at `cpu_hz` (`ns * cpu_hz / 1e9`),
/// never rounding down to zero — a threshold of 0 cycles would classify every
/// pulse the same way and decode an entire frame as ones.
fn ns_to_cycles(ns: u64, cpu_hz: u64) -> u64 {
    ((ns as u128 * cpu_hz as u128) / 1_000_000_000).max(1) as u64
}

/// `nrz_gpio` decode state. Behind a `Mutex` because the observer hook is
/// `&self`.
#[derive(Debug, Default)]
struct NrzState {
    level: bool,
    last_rise: Option<u64>,
    last_fall: Option<u64>,
    shift: u32,
    nbits: u32,
    current: Vec<LedPixel>,
    latched: Vec<LedPixel>,
}

/// The engine device. One per placed strip.
pub struct GenericLedStrip {
    spec: LedStripSpec,
    num_pixels: usize,
    component_id: Option<String>,

    // ── supply ──────────────────────────────────────────────────────────
    /// ABSENT MEANS POWERED — only an explicit `powered: false` from the
    /// diagram compiler darkens a strip. See the APA102 descriptor for the
    /// measurement behind that asymmetry.
    powered: bool,

    // ── spi_frames ──────────────────────────────────────────────────────
    cs_pin: String,
    /// Bytes of the in-flight transaction.
    frame: Vec<u8>,
    /// Latched colours. `spi_frames` only; the NRZ door keeps its own, behind
    /// the mutex, because its hook cannot take `&mut self`.
    pixels: Vec<LedPixel>,

    // ── nrz_gpio ────────────────────────────────────────────────────────
    data_pin: u8,
    high_threshold_cycles: u64,
    reset_threshold_cycles: u64,
    nrz: Mutex<NrzState>,
}

/// Hand-written: a derived `Debug` would print every latched colour, which is a
/// full strip in a panic message. What a reader needs is the strip's IDENTITY
/// and how many LEDs it decoded.
impl std::fmt::Debug for GenericLedStrip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericLedStrip")
            .field("wire", &self.spec.wire)
            .field("num_pixels", &self.num_pixels)
            .field("powered", &self.powered)
            .field("decoded", &self.pixels().len())
            .finish_non_exhaustive()
    }
}

// ─── validation ────────────────────────────────────────────────────────────

/// Validate the static descriptor contract for the `led_strip` primitive.
///
/// Separate from construction so manifest preflight rejects a malformed pack
/// that no canvas has placed yet.
pub(crate) fn validate_descriptor(descriptor: &DeviceDescriptor) -> Result<()> {
    if descriptor.behavior.primitive != "led_strip" {
        bail!(
            "declarative LED strip kit requires behavior.primitive: led_strip, got '{}'",
            descriptor.behavior.primitive
        );
    }
    let spec = descriptor
        .behavior
        .led_strip
        .as_ref()
        .context("declarative LED strip is missing behavior.led_strip")?;
    validate_spec(spec)
}

fn validate_spec(spec: &LedStripSpec) -> Result<()> {
    if spec.default_pixels == 0 {
        bail!("default_pixels is 0 — a strip with no LEDs can never report a colour");
    }
    match spec.wire {
        LedStripWire::NrzGpio => {
            if spec.spi_frames.is_some() {
                bail!(
                    "wire nrz_gpio with an `spi_frames:` block — one strip has one wire, and \
                     which of the two the engine used would be its private business"
                );
            }
            let t = spec
                .timing
                .context("wire nrz_gpio needs a `timing:` block — the bit lengths ARE the part")?;
            if t.high_threshold_ns == 0 {
                bail!(
                    "timing.high_threshold_ns is 0, so every HIGH pulse is longer than it and \
                     the whole frame decodes as ones"
                );
            }
            if t.reset_threshold_ns <= t.high_threshold_ns {
                bail!(
                    "timing.reset_threshold_ns {} is not above high_threshold_ns {} — an \
                     ordinary inter-bit low would latch the frame",
                    t.reset_threshold_ns,
                    t.high_threshold_ns
                );
            }
            if t.bits_per_pixel != 24 {
                bail!(
                    "timing.bits_per_pixel {} — this decoder shifts three colour bytes per LED",
                    t.bits_per_pixel
                );
            }
        }
        LedStripWire::SpiFrames => {
            if spec.timing.is_some() {
                bail!(
                    "wire spi_frames with a `timing:` block — a clocked strip has no bit \
                     durations to read, so the table would gate nothing"
                );
            }
            let f = spec
                .spi_frames
                .as_ref()
                .context("wire spi_frames needs an `spi_frames:` block")?;
            if f.start_frame.is_empty() {
                bail!(
                    "spi_frames.start_frame is empty, so ANY transaction — including a one-byte \
                     glitch — would be decoded as pixels"
                );
            }
            if f.header_mask == 0 {
                bail!(
                    "spi_frames.header_mask is 0: every byte matches, so the end frame decodes \
                     as another LED and the strip grows by four bytes of zeros"
                );
            }
            if f.header_value & !f.header_mask != 0 {
                bail!(
                    "spi_frames.header_value 0x{:02X} has bits outside header_mask 0x{:02X}, so \
                     the comparison can never hold and no frame is ever an LED frame",
                    f.header_value,
                    f.header_mask
                );
            }
            if let Some(b) = f.brightness_mask {
                if b & f.header_mask != 0 {
                    bail!(
                        "spi_frames.brightness_mask 0x{b:02X} overlaps header_mask 0x{:02X}: the \
                         same bits would be both the frame marker and the brightness",
                        f.header_mask
                    );
                }
            }
            if f.colour_bytes.len() != 3 {
                bail!(
                    "spi_frames.colour_bytes names {} bytes; an RGB LED has three",
                    f.colour_bytes.len()
                );
            }
            for i in &f.colour_bytes {
                if u32::from(*i) >= f.frame_bytes {
                    bail!(
                        "spi_frames.colour_bytes index {i} is past the {}-byte LED frame",
                        f.frame_bytes
                    );
                }
            }
        }
    }

    if spec.artifact_meta.is_empty() {
        bail!(
            "artifact_meta is empty: the artifact would carry colours and nothing that explains \
             a dark strip"
        );
    }
    let mut keys: Vec<&str> = Vec::new();
    for field in &spec.artifact_meta {
        let key = field.key();
        if matches!(key, "w" | "h" | "format" | "generation") {
            bail!("artifact_meta publishes '{key}', which describes the payload and is always present");
        }
        if keys.contains(&key) {
            bail!("artifact_meta publishes '{key}' twice");
        }
        keys.push(key);
        match field.flag() {
            LedStripMetaFlag::Brightness | LedStripMetaFlag::CsPin
                if spec.wire != LedStripWire::SpiFrames =>
            {
                bail!("artifact_meta '{key}' is a clocked-SPI fact, but this strip is nrz_gpio");
            }
            LedStripMetaFlag::Brightness
                if spec
                    .spi_frames
                    .as_ref()
                    .is_none_or(|f| f.brightness_mask.is_none()) =>
            {
                bail!(
                    "artifact_meta 'brightness' reads a global-brightness field, and this \
                     strip's frame declares no brightness_mask"
                );
            }
            LedStripMetaFlag::DataPin if spec.wire != LedStripWire::NrzGpio => {
                bail!("artifact_meta '{key}' is a single-wire fact, but this strip is spi_frames");
            }
            LedStripMetaFlag::Powered if !spec.supply_gated => {
                bail!(
                    "artifact_meta 'powered' on a strip that is not `supply_gated` would be a \
                     constant `true` dressed as a measurement"
                );
            }
            _ => {}
        }
    }
    Ok(())
}

// ─── construction ──────────────────────────────────────────────────────────

impl GenericLedStrip {
    pub fn from_descriptor(descriptor: &DeviceDescriptor) -> Result<Self> {
        validate_descriptor(descriptor)?;
        let spec = descriptor
            .behavior
            .led_strip
            .as_ref()
            .context("declarative LED strip is missing behavior.led_strip")?
            .clone();
        Ok(Self::from_spec(spec))
    }

    pub fn from_yaml(yaml: &str) -> Result<Self> {
        Self::from_descriptor(&DeviceDescriptor::from_yaml(yaml)?)
    }

    fn from_spec(spec: LedStripSpec) -> Self {
        let num_pixels = spec.default_pixels.max(1) as usize;
        let mut dev = Self {
            num_pixels,
            component_id: None,
            powered: true,
            cs_pin: String::new(),
            frame: Vec::new(),
            pixels: Vec::new(),
            data_pin: 0,
            high_threshold_cycles: 1,
            reset_threshold_cycles: 1,
            nrz: Mutex::new(NrzState::default()),
            spec,
        };
        dev.set_cpu_hz(160_000_000);
        dev
    }

    /// Scale the descriptor's nanosecond timing to simulated cycles. A no-op on
    /// a clocked strip, which has no durations to read.
    pub fn set_cpu_hz(&mut self, cpu_hz: u64) {
        if let Some(t) = self.spec.timing {
            let hz = cpu_hz.max(1);
            self.high_threshold_cycles = ns_to_cycles(t.high_threshold_ns, hz);
            self.reset_threshold_cycles = ns_to_cycles(t.reset_threshold_ns, hz);
        }
    }

    pub fn set_num_pixels(&mut self, n: usize) {
        self.num_pixels = n.max(1);
    }

    pub fn set_component_id(&mut self, id: impl Into<String>) {
        self.component_id = Some(id.into());
    }

    pub fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    pub fn set_cs_pin(&mut self, pin: impl Into<String>) {
        self.cs_pin = pin.into();
    }

    pub fn set_data_pin(&mut self, pin: u8) {
        self.data_pin = pin;
    }

    pub fn data_pin(&self) -> u8 {
        self.data_pin
    }

    /// Declare whether the strip's supply is connected. Only ever called with
    /// `false`, and only when the compiled manifest says the supply pins are on
    /// no net.
    pub fn set_powered(&mut self, powered: bool) {
        self.powered = powered;
    }

    pub fn powered(&self) -> bool {
        self.powered
    }

    pub fn num_pixels(&self) -> usize {
        self.num_pixels
    }

    /// The strip's displayed colours.
    ///
    /// On the single wire this is the last reset-latched frame once one has
    /// completed, otherwise the frame currently being received — so a single
    /// frame with no trailing reset is still readable, which is what a one-shot
    /// RMT playback produces.
    pub fn pixels(&self) -> Vec<LedPixel> {
        match self.spec.wire {
            LedStripWire::SpiFrames => self.pixels.clone(),
            LedStripWire::NrzGpio => {
                let s = self.nrz.lock().expect("led strip decode state");
                if s.latched.is_empty() {
                    s.current.clone()
                } else {
                    s.latched.clone()
                }
            }
        }
    }

    /// The artifact payload: colour bytes in WIRE order, LED after LED.
    fn flat(&self) -> Vec<u8> {
        self.pixels().iter().flat_map(|p| p.wire).collect()
    }

    // ── the clocked wire ────────────────────────────────────────────────

    /// Parse a completed transaction. Requires the start frame and at least one
    /// well-formed LED frame; anything else leaves the previous colours
    /// untouched, because a glitchy transaction must not blank a strip.
    fn latch(&mut self) {
        let Some(f) = self.spec.spi_frames.as_ref() else {
            return;
        };
        let start = f.start_frame.len();
        let stride = f.frame_bytes as usize;
        if self.frame.len() < start + stride || self.frame[..start] != f.start_frame[..] {
            return;
        }
        let mut out: Vec<LedPixel> = Vec::new();
        for chunk in self.frame[start..].chunks_exact(stride) {
            if out.len() >= self.num_pixels {
                break;
            }
            if chunk[0] & f.header_mask != f.header_value {
                break; // end frame or garbage — stop decoding
            }
            let brightness = f.brightness_mask.map_or(0, |m| chunk[0] & m);
            let wire = [
                chunk[f.colour_bytes[0] as usize],
                chunk[f.colour_bytes[1] as usize],
                chunk[f.colour_bytes[2] as usize],
            ];
            out.push(LedPixel { wire, brightness });
        }
        if !out.is_empty() {
            self.pixels = out;
        }
    }

    // ── the single wire ─────────────────────────────────────────────────

    /// Feed one pad transition. Decodes bit HIGH durations into LEDs and
    /// detects the reset/latch gap. A no-op for edges on another pad or for a
    /// non-transition.
    fn on_edge(&self, pin: u8, to: bool, sim_cycle: u64) {
        if pin != self.data_pin {
            return;
        }
        let bits = self.spec.timing.map_or(24, |t| t.bits_per_pixel);
        let mut s = self.nrz.lock().expect("led strip decode state");
        if to == s.level {
            return;
        }
        s.level = to;
        if to {
            // Rising edge — the start of a bit's HIGH. A long preceding LOW is
            // the reset gap: display the frame just received and start fresh.
            if let Some(fall) = s.last_fall {
                if sim_cycle.saturating_sub(fall) >= self.reset_threshold_cycles {
                    if !s.current.is_empty() {
                        s.latched = std::mem::take(&mut s.current);
                    }
                    s.current.clear();
                    s.shift = 0;
                    s.nbits = 0;
                }
            }
            s.last_rise = Some(sim_cycle);
        } else {
            // Falling edge — the HIGH duration decides this bit.
            if let Some(rise) = s.last_rise {
                let high = sim_cycle.saturating_sub(rise);
                let bit = u32::from(high > self.high_threshold_cycles);
                s.shift = (s.shift << 1) | bit;
                s.nbits += 1;
                if s.nbits == bits {
                    let wire = [
                        ((s.shift >> 16) & 0xFF) as u8,
                        ((s.shift >> 8) & 0xFF) as u8,
                        (s.shift & 0xFF) as u8,
                    ];
                    if s.current.len() < self.num_pixels {
                        s.current.push(LedPixel {
                            wire,
                            brightness: 0,
                        });
                    }
                    s.shift = 0;
                    s.nbits = 0;
                }
            }
            s.last_fall = Some(sim_cycle);
        }
    }

    // ── the artifact ────────────────────────────────────────────────────

    fn build_artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        let pixels = self.pixels();
        let flat = self.flat();
        let mut meta = serde_json::Map::new();
        // `w` is the strip's OWN idea of its extent, which the two wires read
        // differently and deliberately: a clocked strip reports what it latched
        // (an unpowered one is zero LEDs wide), a single-wire strip reports its
        // configured length whatever the decoder saw. Both are the published
        // contract of the model this replaces.
        let w = match self.spec.wire {
            LedStripWire::SpiFrames => pixels.len(),
            LedStripWire::NrzGpio => self.num_pixels,
        };
        meta.insert("w".into(), serde_json::json!(w));
        meta.insert("h".into(), serde_json::json!(1));
        meta.insert(
            "format".into(),
            serde_json::json!(self.spec.artifact_format),
        );
        meta.insert(
            "generation".into(),
            serde_json::json!(crate::inspect::artifact_generation(&flat)),
        );
        for field in &self.spec.artifact_meta {
            let value = match field.flag() {
                LedStripMetaFlag::Brightness => {
                    serde_json::json!(pixels.iter().map(|p| p.brightness).collect::<Vec<u8>>())
                }
                LedStripMetaFlag::PixelsDecoded => serde_json::json!(pixels.len()),
                LedStripMetaFlag::LitPixels => serde_json::json!(pixels
                    .iter()
                    .filter(|p| p.wire.iter().any(|&c| c != 0))
                    .count()),
                LedStripMetaFlag::Powered => serde_json::json!(self.powered),
                LedStripMetaFlag::CsPin => serde_json::json!(self.cs_pin),
                LedStripMetaFlag::DataPin => serde_json::json!(self.data_pin),
            };
            meta.insert(field.key().to_string(), value);
        }
        vec![crate::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::Value::Object(meta),
            bytes: crate::inspect::artifact_bytes(&flat, opts),
        }]
    }
}

// ─── the clocked-SPI door ──────────────────────────────────────────────────

impl SpiDevice for GenericLedStrip {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn cs_select(&mut self) {
        self.frame.clear();
    }

    fn cs_release(&mut self) {
        self.latch();
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // THE ONE GATE THAT MAKES AN UNPOWERED STRIP BEHAVE LIKE ONE. Every
        // byte of every frame arrives here and `latch` only ever reads what
        // this pushed, so refusing the wire leaves the colours empty BY
        // CONSTRUCTION rather than masking them at report time.
        if !self.powered {
            return 0;
        }
        self.frame.push(mosi);
        0 // MISO is never driven
    }

    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        self.build_artifacts(id, opts)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    // Both halves, deliberately. A one-sided impl compiles and passes every
    // unit test while the downcast that reads the colours from outside silently
    // gets None.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

// ─── the single-wire door ──────────────────────────────────────────────────

impl crate::peripherals::device::GpioObserver for GenericLedStrip {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_edge(pin, to, sim_cycle);
    }
}

/// A strip is held by the bus so the UI and the oracle can read the decoded
/// colours back, and it IS a display surface, so it reports its own artifact.
impl crate::inspect::DeviceEvidence for GenericLedStrip {
    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        self.build_artifacts(id, opts)
    }
}

/// The bus holds a single-wire strip purely so `inspect` can read it back; its
/// manifest id is the identity the author wrote, not the part name.
impl crate::bus::ObservedDevice for GenericLedStrip {
    fn manifest_id(&self) -> &str {
        self.component_id().unwrap_or("led_strip")
    }

    fn evidence(&self) -> Option<&dyn crate::inspect::DeviceEvidence> {
        Some(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_arc_any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

/// A [`PeripheralKit`] backed by a declarative `led_strip` descriptor.
pub struct DeclarativeLedStripKit {
    descriptor: DeviceDescriptor,
    metadata: &'static KitMetadata,
}

impl DeclarativeLedStripKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        validate_descriptor(&descriptor)?;
        let metadata = leak_metadata(&descriptor);
        Ok(Self {
            descriptor,
            metadata,
        })
    }

    fn spec(&self) -> &LedStripSpec {
        self.descriptor
            .behavior
            .led_strip
            .as_ref()
            .expect("validated at construction")
    }
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn config_type_from_str(ty: &str) -> ConfigType {
    match ty {
        "int" => ConfigType::Int,
        "float" => ConfigType::Float,
        "bool" => ConfigType::Bool,
        _ => ConfigType::Str,
    }
}

fn leak_metadata(descriptor: &DeviceDescriptor) -> &'static KitMetadata {
    let spec = descriptor
        .behavior
        .led_strip
        .as_ref()
        .expect("validated at construction");
    let meta = descriptor.metadata.as_ref();
    let label = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| descriptor.r#type.clone());
    let summary = meta
        .and_then(|m| m.summary.clone())
        .unwrap_or_else(|| "Declarative LED strip.".to_string());
    let detail = meta
        .and_then(|m| m.detail.clone())
        .unwrap_or_else(|| summary.clone());
    let config_keys: &'static [ConfigKey] = Box::leak(
        meta.map(|m| m.config_keys.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|k| ConfigKey {
                name: leak(k.name.clone()),
                ty: config_type_from_str(&k.ty),
                doc: leak(k.doc.clone()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let labs: &'static [LabRef] = Box::leak(
        meta.map(|m| m.labs.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|l| LabRef {
                board_id: leak(l.board_id.clone()),
                chip: leak(l.chip.clone()),
                example_dir: leak(l.example_dir.clone()),
                demo_elf: leak(l.demo_elf.clone()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let (transport, category) = match spec.wire {
        LedStripWire::SpiFrames => (Transport::Spi, Category::Spi),
        LedStripWire::NrzGpio => (Transport::GpioGroup, Category::Gpio),
    };
    Box::leak(Box::new(KitMetadata {
        device_type: leak(descriptor.r#type.clone()),
        label: leak(label),
        summary: leak(summary),
        detail: leak(detail),
        transport,
        category,
        config_keys,
        labs,
        inputs: &[],
    }))
}

impl PeripheralKit for DeclarativeLedStripKit {
    fn metadata(&self) -> &'static KitMetadata {
        self.metadata
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let spec = self.spec();
        let mut dev = GenericLedStrip::from_descriptor(&self.descriptor)?;
        dev.set_num_pixels(
            ctx.config_i64("num_pixels")
                .unwrap_or(i64::from(spec.default_pixels))
                .max(1) as usize,
        );
        dev.set_component_id(ctx.device_id().to_string());
        if spec.supply_gated && ctx.config_bool("powered") == Some(false) {
            dev.set_powered(false);
        }
        match spec.wire {
            LedStripWire::SpiFrames => {
                dev.set_cs_pin(ctx.config_str("cs_pin").unwrap_or("PA4").to_string());
                ctx.attach_spi_device(Box::new(dev))
            }
            LedStripWire::NrzGpio => {
                let label = ctx.config_str("data_pin").unwrap_or("GPIO48").to_string();
                let pin = ctx.parse_gpio_pin(&label).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{} '{}' data_pin '{}' could not be parsed to an ESP GPIO (0..=48)",
                        self.descriptor.r#type,
                        ctx.device_id(),
                        label
                    )
                })?;
                dev.set_data_pin(pin);
                // The placed part's own `cpu_hz` first, then the system's clock
                // (manifest override, else the chip descriptor), and only then
                // the historical literal — which is what every board used to
                // get, S3 and ATmega included.
                let cpu_hz = ctx
                    .config_i64("cpu_hz")
                    .map(|v| v as u64)
                    .or_else(|| Some(ctx.bus.cpu_hz).filter(|hz| *hz > 0))
                    .unwrap_or(160_000_000);
                dev.set_cpu_hz(cpu_hz);
                let strip = std::sync::Arc::new(dev);
                // Classic ESP32 + S3 GPIO observers (the same choke motors and
                // the parallel TFT use).
                ctx.install_gpio_observer(strip.clone());
                ctx.bus.observe_device(strip);
                Ok(())
            }
        }
    }
}

// ─── Registry statics ──────────────────────────────────────────────────────

use std::sync::LazyLock;

impl PeripheralKit for LazyLock<DeclarativeLedStripKit> {
    fn metadata(&self) -> &'static KitMetadata {
        LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        LazyLock::force(self).attach(ctx)
    }
}

/// Build a declarative LED strip straight from its embedded descriptor. Used by
/// the engine's own tests and by any caller that wants the model without going
/// through a manifest.
pub fn embedded(device_type: &str) -> Result<GenericLedStrip> {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .with_context(|| format!("no embedded descriptor for '{device_type}'"))?;
    GenericLedStrip::from_yaml(yaml)
}

macro_rules! led_strip_kit {
    ($(#[$meta:meta])* $ident:ident, $type:literal) => {
        $(#[$meta])*
        pub static $ident: LazyLock<DeclarativeLedStripKit> = LazyLock::new(|| {
            DeclarativeLedStripKit::from_yaml(
                labwired_config::embedded_device_yaml($type)
                    .expect(concat!($type, " descriptor embedded")),
            )
            .expect(concat!($type, " is a valid declarative LED strip descriptor"))
        });
    };
}

led_strip_kit!(
    /// APA102 / DotStar, clocked SPI (`apa102.yaml`).
    APA102_KIT,
    "apa102"
);
led_strip_kit!(
    /// WS2812 / WS2812B / SK6812 NeoPixel, single wire (`ws2812.yaml`).
    WS2812_KIT,
    "neopixel"
);

/// The APA102 model with its CS pin and length wired, for tests that drive the
/// wire directly rather than through a manifest. The shape the in-crate tests
/// used to get from `Apa102::new`.
pub fn apa102(cs_pin: &str, num_pixels: usize) -> GenericLedStrip {
    let mut dev = embedded("apa102").expect("apa102 descriptor builds");
    dev.set_cs_pin(cs_pin);
    dev.set_num_pixels(num_pixels);
    dev
}

/// The WS2812 model on one pad, with the firmware clock its timing scales from.
/// The shape the in-crate tests used to get from `Ws2812::new`.
pub fn ws2812(pin: u8, num_pixels: usize, cpu_hz: u64) -> GenericLedStrip {
    let mut dev = embedded("neopixel").expect("neopixel descriptor builds");
    dev.set_data_pin(pin);
    dev.set_num_pixels(num_pixels);
    dev.set_cpu_hz(cpu_hz);
    dev
}

#[cfg(test)]
#[path = "declarative_led_strip_tests.rs"]
mod tests;
