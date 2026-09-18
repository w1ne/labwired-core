// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic **declarative display** — one engine device that interprets a
//! [`labwired_config::DisplaySpec`], so a framebuffer panel is a YAML file
//! rather than eight hundred lines of Rust.
//!
//! Why a primitive and not another `spi_device`
//! ===========================================
//! Fifteen of the engine's eighty-two device models are displays, and they are
//! the least register-like parts in the tree: a display controller has no
//! register map worth the name, it has an ADDRESS COUNTER and a command table
//! that moves it. Expressing that as `registers:` would have meant writing the
//! counter arithmetic once per part, in Rust, which is exactly what the six
//! panel models in `components/` already were.
//!
//! What is data
//! ------------
//! Geometry, pixel format, RAM layout, the command/data framing (a D/C pad on
//! 4-wire SPI; a control byte on I²C), the command table, and how the counters
//! wrap per addressing mode. That is everything a controller datasheet states.
//!
//! What is engine
//! --------------
//! This file: the counter arithmetic, the window wrap, the parameter state
//! machine, the orientation map, and the paint artifact. One implementation,
//! shared by every panel, instead of one per panel.
//!
//! What is deliberately NOT modelled
//! ---------------------------------
//! Pixel-value transforms. Contrast, gamma and inversion are recorded as panel
//! FLAGS and reported in the artifact's `meta`; they are never applied to the
//! stored bytes. A twin that pre-rendered its own idea of the picture could not
//! be compared against a photograph of the glass, and the whole point of the
//! byte-exact artifact is that it can.
//!
//! One struct, two buses
//! ---------------------
//! [`GenericDisplay`] implements BOTH [`I2cDevice`] and [`SpiDevice`], because
//! the SSD1306 and the ST7789 differ in framing and in nothing else: the same
//! command table, counters and artifact serve a panel addressed by a control
//! byte and a panel addressed by a pad. `behavior.display.dc.source` picks
//! which door the part uses, and the attach path routes it accordingly.

use std::any::Any;
use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use labwired_config::{
    DeviceDescriptor, DisplayAction, DisplayAddressingMode, DisplayAxis, DisplayCommand,
    DisplayCsSelect, DisplayCursorPart, DisplayDcSource, DisplayDcUnwired, DisplayMetaFlag,
    DisplayMetaFormat, DisplayPageWrap, DisplayPixelFormat, DisplayPlaneOf, DisplayRamLayout,
    DisplayRamStream, DisplaySpec, DisplayValue,
};

use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::spi::SpiDevice;

/// A glass that shows a strip of the controller's frame memory, in FIXED
/// physical coordinates. Its origin does not move when firmware changes the
/// orientation byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlassWindow {
    pub col_offset: u16,
    pub row_offset: u16,
    pub cols: u16,
    pub rows: u16,
}

/// A borrowed, by-name view of a multi-plane panel's frame memory and glass.
/// See [`GenericDisplay::planes`].
pub struct PlaneView<'a> {
    dev: &'a GenericDisplay,
}

impl PlaneView<'_> {
    /// Plane names in payload order. EMPTY for a panel with one undivided
    /// frame memory — the honest answer, and what tells a caller to take its
    /// single-framebuffer branch.
    pub fn names(&self) -> &[String] {
        &self.dev.plane_names
    }

    /// Bytes in one plane.
    pub fn plane_bytes(&self) -> usize {
        self.dev.plane_bytes
    }

    /// The value an erased byte holds, and therefore what "no ink" is.
    pub fn blank(&self) -> u8 {
        self.dev.spec.ram.blank
    }

    fn slice(buf: &[u8], idx: usize, len: usize) -> &[u8] {
        &buf[idx * len..(idx + 1) * len]
    }

    /// One plane of FRAME MEMORY, as the wire wrote it.
    pub fn ram(&self, name: &str) -> Option<&[u8]> {
        let i = self.dev.plane_names.iter().position(|p| p == name)?;
        Some(Self::slice(&self.dev.ram, i, self.dev.plane_bytes))
    }

    /// One plane of THE GLASS — what the last `refresh` latched. Not the same
    /// thing as [`Self::ram`] on an e-paper, which is the whole point.
    pub fn screen(&self, name: &str) -> Option<&[u8]> {
        let i = self.dev.plane_names.iter().position(|p| p == name)?;
        Some(Self::slice(&self.dev.screen, i, self.dev.plane_bytes))
    }

    /// Inked bytes of one plane of frame memory — bytes that are not
    /// [`Self::blank`].
    pub fn ink_bytes(&self, name: &str) -> Option<usize> {
        let blank = self.blank();
        Some(self.ram(name)?.iter().filter(|&&b| b != blank).count())
    }
}

/// Which of the two real D/C wirings a placement uses. Only `hw_dcx` panels
/// have a choice; a `pin` panel is always [`DcWiring::Gpio`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DcWiring {
    /// A GPIO the firmware toggles between transfers; the bus latches that
    /// pin's output register.
    Gpio,
    /// The SPI controller's own DCX line (nRF54L `PSEL.DCX` + `DCXCNT`).
    ControllerDcx,
}

impl DcWiring {
    /// The string the artifact publishes. The RM67162's contract.
    fn as_str(self) -> &'static str {
        match self {
            Self::Gpio => "gpio",
            Self::ControllerDcx => "controller_dcx",
        }
    }
}

/// Where the next wire byte goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    /// No command is open. A data byte here is a stray, which is what it is on
    /// silicon too.
    Idle,
    /// A command is collecting its parameters.
    Params,
    /// The RAM write stream is open; data bytes are pixels.
    Ram,
}

/// The engine device. One per placed panel.
pub struct GenericDisplay {
    // ── the descriptor, compiled ────────────────────────────────────────
    spec: DisplaySpec,
    /// `opcode → the command-table entries that claim it`, in declaration
    /// order. Built once so decoding a byte is an array index rather than a
    /// table scan. A list rather than one index because an instruction-set
    /// bank (`when:`) lets two entries claim the same opcode under mutually
    /// exclusive guards — the PCD8544's `0x80|n` is "set X" in the basic set
    /// and "set Vop" in the extended one.
    by_opcode: Box<[Vec<u16>; 256]>,
    width: usize,
    height: usize,
    pages: usize,
    unit_bytes: usize,
    /// Addressable extent in COUNTER STEPS, not pixels. Equal to `width` for
    /// every panel whose column counter steps one pixel; `width / 8` for the
    /// SSD1680, whose 0x44 window bounds are byte coordinates. See
    /// `DisplayRam::units`.
    col_units: usize,
    row_units: usize,
    /// Bytes in ONE plane. Equal to the whole frame memory when the panel has
    /// no planes.
    plane_bytes: usize,
    /// Plane names in payload order, empty for a single-plane panel.
    plane_names: Vec<String>,

    // ── identity ────────────────────────────────────────────────────────
    address: u8,
    cs_pin: String,
    dc_pin: Option<String>,
    dc_source: Option<(u64, u8)>,
    dc_level: bool,
    dc_wiring: DcWiring,
    component_id: Option<String>,

    // ── supply ──────────────────────────────────────────────────────────
    /// Whether the module's supply pins are connected in the design. ABSENT
    /// MEANS POWERED — only an explicit `powered: false` from the diagram
    /// compiler darkens a panel. See the ST7789 descriptor for the measurement
    /// that forced that default.
    powered: bool,

    // ── panel state ─────────────────────────────────────────────────────
    display_on: bool,
    awake: bool,
    inverted: bool,
    mode: DisplayAddressingMode,
    vars: BTreeMap<String, u32>,

    col: u16,
    row: u16,
    page: u16,
    col_start: u16,
    col_end: u16,
    row_start: u16,
    row_end: u16,
    page_start: u16,
    page_end: u16,

    ram: Vec<u8>,
    /// WHAT IS ON THE GLASS. Equal to `ram` for every panel whose frame memory
    /// IS the screen; latched from `ram` by a `refresh` action on the panels
    /// where it is not (e-paper). Same length as `ram` always, so a consumer
    /// can index it the same way.
    screen: Vec<u8>,
    /// How many `refresh` actions have run. Never reset by anything.
    refresh_generation: u32,

    // ── protocol state ──────────────────────────────────────────────────
    framing: Framing,
    pending_cmd: u8,
    /// The command-table entry the pending opcode resolved to. Held rather
    /// than re-looked-up when the parameters complete, because a command may
    /// change the very var its own guard reads.
    pending_idx: Option<u16>,
    params: [u8; 64],
    param_have: u8,
    param_want: u8,
    unit: [u8; 4],
    unit_have: usize,
    /// Which plane the open RAM stream writes, as an index into `plane_names`.
    /// 0 for a single-plane panel, which is the whole frame memory.
    plane: usize,
    /// Write units the open stream may still accept under
    /// `ram.stream: window_counted`. `None` = the stream is not counted.
    ram_remaining: Option<u32>,
    /// I²C only: the control byte latched at the start of this transaction.
    /// `None` = the next byte IS the control byte.
    control: Option<u8>,

    glass: Option<GlassWindow>,
    elapsed_us: u64,
}

/// Deliberately hand-written: a derived `Debug` would print the frame memory,
/// which is up to 150 KB of pixels in a panic message. What a reader needs is
/// the panel's IDENTITY and its control state — the things that decide where a
/// byte went.
impl std::fmt::Debug for GenericDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericDisplay")
            .field("format", &self.spec.pixel_format)
            .field("extent", &(self.width, self.height))
            .field("ram_bytes", &self.ram.len())
            .field("powered", &self.powered)
            .field("display_on", &self.display_on)
            .field("awake", &self.awake)
            .field("inverted", &self.inverted)
            .field("dc_wiring", &self.dc_wiring)
            .field("mode", &self.mode)
            .field("vars", &self.vars)
            .field("cursor", &(self.col, self.row, self.page))
            .field(
                "window",
                &(
                    (self.col_start, self.col_end),
                    (self.row_start, self.row_end),
                    (self.page_start, self.page_end),
                ),
            )
            .field("framing", &self.framing)
            .finish_non_exhaustive()
    }
}

// ─── validation ────────────────────────────────────────────────────────────

/// Validate the static descriptor contract for the `display` primitive.
///
/// Separate from construction so manifest preflight rejects a malformed pack
/// that no canvas has placed yet — a saved manifest is a complete portable
/// catalog, not a set of deferred surprises.
pub(crate) fn validate_descriptor(descriptor: &DeviceDescriptor) -> Result<()> {
    if descriptor.behavior.primitive != "display" {
        bail!(
            "declarative display kit requires behavior.primitive: display, got '{}'",
            descriptor.behavior.primitive
        );
    }
    let spec = descriptor
        .behavior
        .display
        .as_ref()
        .context("declarative display is missing behavior.display")?;
    validate_spec(spec)
}

fn validate_spec(spec: &DisplaySpec) -> Result<()> {
    if spec.width == 0 || spec.height == 0 {
        bail!(
            "display geometry is {}x{} — a panel has extent",
            spec.width,
            spec.height
        );
    }
    let w = spec.width as usize;
    let h = spec.height as usize;

    // The stated RAM size is CHECKED against the geometry rather than trusted.
    // Two numbers for one fact is how a descriptor starts lying: a `bytes:`
    // that does not match the panel would allocate a buffer the counters walk
    // off the end of, and every write past it would be silently dropped.
    // Counter STEPS, which is what the frame memory is indexed by. A byte-unit
    // axis addresses eight pixels per step (SSD1680 0x44).
    let step_w = spec.ram.units.col.pixels_per_step() as usize;
    let step_h = spec.ram.units.row.pixels_per_step() as usize;
    if step_w > 1 || step_h > 1 {
        if spec.pixel_format.write_unit_bytes() != 1 {
            bail!(
                "ram.units counts bytes on a {:?} panel, whose write unit is {} bytes — one                  counter step cannot be both",
                spec.pixel_format,
                spec.pixel_format.write_unit_bytes()
            );
        }
        if w % step_w != 0 || h % step_h != 0 {
            bail!(
                "ram.units divides the {w}x{h} geometry into {step_w}x{step_h}-pixel steps,                  which does not fit a whole number of steps"
            );
        }
        if spec.orientation.is_some() || spec.glass_crop {
            bail!(
                "ram.units counts bytes AND the panel declares an orientation or a glass crop;                  this engine maps a crop and a rotation in pixels, so the two would disagree                  about what a coordinate means"
            );
        }
    }
    let units_w = w / step_w;
    let units_h = h / step_h;

    // PLANES. Two 1-bpp RAMs selected by the command that opens the stream.
    let planes = &spec.ram.planes;
    if !planes.is_empty() {
        if spec.pixel_format.write_unit_bytes() != 1 {
            bail!(
                "ram.planes divides frame memory into 1 bpp planes, but the pixel format is                  {:?}, whose write unit is {} bytes",
                spec.pixel_format,
                spec.pixel_format.write_unit_bytes()
            );
        }
        if planes.len() < 2 {
            bail!(
                "ram.planes lists one plane ('{}'); one plane is an undivided frame memory,                  which is what an empty list already says",
                planes[0]
            );
        }
        let mut seen: Vec<&str> = Vec::new();
        for name in planes {
            if name.is_empty() {
                bail!("ram.planes carries an unnamed plane; a ram_write selects a plane BY NAME");
            }
            if seen.contains(&name.as_str()) {
                bail!("ram.planes lists '{name}' twice — a ram_write naming it could mean either");
            }
            seen.push(name);
        }
    }

    let derived = match (spec.pixel_format, spec.ram.layout) {
        (DisplayPixelFormat::MonoPage, DisplayRamLayout::PageMajor) => {
            let pages = spec
                .ram
                .pages
                .context("a page-major mono panel must state ram.pages")?
                as usize;
            if pages * 8 != h {
                bail!(
                    "ram.pages {pages} × 8 rows = {} but the panel is {h} rows tall",
                    pages * 8
                );
            }
            units_w * pages * planes.len().max(1)
        }
        (DisplayPixelFormat::MonoPage, DisplayRamLayout::RowMajor) => {
            bail!("mono_page pixels are page-major by construction; ram.layout says row_major")
        }
        (fmt, DisplayRamLayout::RowMajor) => {
            units_w * units_h * fmt.write_unit_bytes() * planes.len().max(1)
        }
        (fmt, DisplayRamLayout::PageMajor) => {
            bail!("{fmt:?} with ram.layout page_major is not a shape this engine knows")
        }
    };
    if spec.ram.bytes as usize != derived {
        bail!(
            "ram.bytes {} does not match the {}x{} {:?} geometry ({} bytes)",
            spec.ram.bytes,
            spec.width,
            spec.height,
            spec.pixel_format,
            derived
        );
    }

    match spec.dc.source {
        DisplayDcSource::Pin | DisplayDcSource::HwDcx => {
            if spec.dc.command_level > 1 {
                bail!("dc.command_level {} is not a level", spec.dc.command_level);
            }
            if spec.default_address.is_some() {
                bail!("a D/C-pin panel is selected by CS, not by an I²C address");
            }
        }
        DisplayDcSource::ControlByte => {
            if spec.dc.command_value.is_none() {
                bail!("dc.source control_byte needs a dc.command_value (the byte that opens the command stream)");
            }
            if spec.default_address.is_none() {
                bail!("a control-byte panel sits on I²C and needs a default_address");
            }
        }
    }

    if !spec.addressing.modes.contains(&spec.addressing.default) {
        bail!(
            "addressing.default {:?} is not in addressing.modes {:?}",
            spec.addressing.default,
            spec.addressing.modes
        );
    }
    if spec.ram.layout == DisplayRamLayout::RowMajor {
        for m in &spec.addressing.modes {
            if *m != DisplayAddressingMode::Horizontal {
                bail!("addressing mode {m:?} has no meaning on a row-major frame memory");
            }
        }
    }

    if let Some(o) = &spec.orientation {
        if !spec.vars.contains_key(&o.var) {
            bail!(
                "orientation.var '{}' is not declared in vars — the orientation would read a \
                 cell nothing can write, so every rotation command would silently do nothing",
                o.var
            );
        }
    }

    // A data byte reaches frame memory unconditionally, or after a `ram_write`.
    // The two must agree with the command table, or a panel silently drops
    // every pixel it is sent.
    let has_ram_write = spec
        .commands
        .iter()
        .flat_map(|c| c.actions.iter())
        .any(|a| a.ram_write.is_some());
    match spec.ram.stream {
        DisplayRamStream::Always => {
            if has_ram_write {
                bail!(
                    "ram.stream `always` means every data byte is frame memory, but the command \
                     table declares a `ram_write` — one of the two is describing a different part"
                );
            }
            if spec.dc.source != DisplayDcSource::ControlByte
                && spec.commands.iter().any(|c| c.args > 0)
            {
                bail!(
                    "a D/C-pad panel whose data line is ALWAYS frame memory has nowhere to put a \
                     command's parameters: command 0x{:02X} declares {} of them",
                    spec.commands
                        .iter()
                        .find(|c| c.args > 0)
                        .map(|c| c.opcode)
                        .unwrap_or(0),
                    spec.commands
                        .iter()
                        .find(|c| c.args > 0)
                        .map(|c| c.args)
                        .unwrap_or(0),
                );
            }
        }
        DisplayRamStream::Command | DisplayRamStream::WindowCounted => {
            if !has_ram_write {
                bail!(
                    "ram.stream `{:?}` means a `ram_write` action opens the pixel stream, and \
                     the command table declares none — no data byte could ever reach frame memory",
                    spec.ram.stream
                );
            }
        }
    }

    // THE UNWIRED-D/C CHEAT. Stated per panel; see `DisplayDc::unwired`.
    match spec.dc.unwired {
        DisplayDcUnwired::Level => {}
        DisplayDcUnwired::Infer | DisplayDcUnwired::Data => {
            if spec.dc.source == DisplayDcSource::ControlByte {
                bail!(
                    "dc.unwired {:?} describes a panel with no D/C PAD resolved, and this panel \
                     is framed by an I²C control byte, which has no pad to be missing",
                    spec.dc.unwired
                );
            }
        }
    }
    if spec.dc.unwired == DisplayDcUnwired::Infer
        && spec.ram.stream != DisplayRamStream::WindowCounted
    {
        bail!(
            "dc.unwired `infer` decides a byte is a command because NO STREAM IS OPEN, and \
             ram.stream `{:?}` leaves the pixel stream open until the next command — which \
             under this cheat can never arrive, so every byte after the first ram_write would \
             be a pixel forever",
            spec.ram.stream
        );
    }

    // `refresh` and the counter that reports it must both exist or neither.
    let has_refresh = spec
        .commands
        .iter()
        .flat_map(|c| c.actions.iter())
        .any(|a| a.refresh);
    let publishes_generation = spec
        .artifact_meta
        .iter()
        .any(|f| f.flag() == Some(DisplayMetaFlag::RefreshGeneration));
    if publishes_generation && !has_refresh {
        bail!(
            "artifact_meta publishes 'refresh_generation' and the command table has no \
             `refresh` action — the key would report 0 for every firmware forever"
        );
    }
    if has_refresh && !publishes_generation {
        bail!(
            "the command table declares a `refresh` action and artifact_meta publishes no \
             'refresh_generation' — nothing could tell a written frame from a shown one, \
             which is the only reason the action exists"
        );
    }
    if has_refresh
        && spec
            .artifact_meta
            .iter()
            .all(|f| f.plane().is_none_or(|(_, of)| of != DisplayPlaneOf::Screen))
        && !spec.ram.planes.is_empty()
    {
        bail!(
            "a multi-plane panel with a `refresh` action publishes no `of: screen` plane \
             count — frame memory and the glass would be indistinguishable in the artifact, \
             so firmware that wrote a frame and never activated would read as if it had"
        );
    }

    for req in &spec.lit_requires {
        if !spec.vars.contains_key(&req.var) {
            bail!(
                "lit_requires reads var '{}', which is not declared — the clause would read a \
                 cell nothing can write, so the panel would be dark for every firmware",
                req.var
            );
        }
        if req.min == 0 {
            bail!(
                "lit_requires '{}' min: 0 — every value satisfies it, so the clause gates \
                 nothing while reading as if it did",
                req.var
            );
        }
    }

    if spec.artifact_meta.is_empty() {
        bail!(
            "artifact_meta is empty: the paint artifact would carry pixels and no panel state, \
             so a dark frame could not explain itself"
        );
    }
    let mut meta_keys: Vec<String> = Vec::new();
    for field in &spec.artifact_meta {
        let key = field.key().into_owned();
        if matches!(key.as_str(), "w" | "h" | "format" | "generation") {
            bail!("artifact_meta publishes '{key}', which describes the payload and is always present");
        }
        if meta_keys.contains(&key) {
            bail!("artifact_meta publishes '{key}' twice");
        }
        meta_keys.push(key.clone());
        if let Some(var) = field.var() {
            if !spec.vars.contains_key(var) {
                bail!(
                    "artifact_meta publishes var '{var}', which is not declared — the key would \
                     read a cell nothing can write and report a constant forever"
                );
            }
            continue;
        }
        if let Some((plane, _)) = field.plane() {
            if !spec.ram.planes.iter().any(|p| p == plane) {
                bail!(
                    "artifact_meta counts plane '{plane}', which ram.planes does not declare — \
                     the key would report 0 forever"
                );
            }
            continue;
        }
        match field.flag() {
            Some(DisplayMetaFlag::InkBytes) | Some(DisplayMetaFlag::LitPixels)
                if spec.pixel_format != DisplayPixelFormat::MonoPage =>
            {
                bail!(
                    "artifact_meta '{key}' counts 1 bpp ink, but this panel is {:?}",
                    spec.pixel_format
                );
            }
            Some(DisplayMetaFlag::TopColour) | Some(DisplayMetaFlag::TopColourPixels)
                if spec.pixel_format != DisplayPixelFormat::Rgb565 =>
            {
                bail!(
                    "artifact_meta '{key}' reads a 16-bit pixel, but this panel is {:?}",
                    spec.pixel_format
                );
            }
            Some(DisplayMetaFlag::PlaneBytes) if spec.ram.planes.is_empty() => {
                bail!(
                    "artifact_meta publishes 'plane_bytes', the SPLIT of a multi-plane payload, \
                     and ram.planes declares none — there is nothing to split"
                );
            }
            Some(DisplayMetaFlag::DcSource) if spec.dc.source != DisplayDcSource::HwDcx => {
                bail!(
                    "artifact_meta 'dc_source' names WHICH of two wirings drives D/C, and \
                     dc.source {:?} admits only one — the key would be a constant",
                    spec.dc.source
                );
            }
            _ => {}
        }
    }

    let mut claims: Vec<Vec<usize>> = vec![Vec::new(); 256];
    for (i, cmd) in spec.commands.iter().enumerate() {
        // 64 rather than 8: the UC8151D's waveform LUT commands (0x20 VCOM, 44
        // bytes; 0x21..=0x24 WW/BW/WB/BB, 42 each) are real entries in a real
        // command table, and a table that could not state their length would
        // have to leave them undeclared and rely on an unlisted opcode dropping
        // its parameters — which is only a no-op while `ram.stream` is
        // `command`.
        if cmd.args as usize > 64 {
            bail!(
                "command 0x{:02X} takes {} parameters; this engine buffers 64",
                cmd.opcode,
                cmd.args
            );
        }
        let end = cmd.opcode_end.unwrap_or(cmd.opcode);
        if end < cmd.opcode {
            bail!(
                "command range 0x{:02X}..=0x{:02X} runs backwards",
                cmd.opcode,
                end
            );
        }
        if let Some(w) = &cmd.when {
            if !spec.vars.contains_key(&w.var) {
                bail!(
                    "command 0x{:02X}: `when` reads var '{}', which is not declared — the guard \
                     would read a cell nothing can write and the entry would never decode",
                    cmd.opcode,
                    w.var
                );
            }
        }
        for op in cmd.opcode..=end {
            for &other in &claims[op as usize] {
                if !guards_are_exclusive(&spec.commands[other], cmd) {
                    bail!(
                        "opcode 0x{op:02X} is claimed twice in the command table without \
                         mutually exclusive `when` guards; which entry runs would depend on \
                         declaration order, which is not something a datasheet says"
                    );
                }
            }
            claims[op as usize].push(i);
        }
        for action in &cmd.actions {
            validate_action(spec, cmd, action)?;
        }
    }
    Ok(())
}

/// Two entries may claim one opcode only when their guards cannot both hold:
/// the same var, read through the same mask, required to equal two different
/// values. Anything looser and the decode depends on declaration order.
fn guards_are_exclusive(a: &DisplayCommand, b: &DisplayCommand) -> bool {
    match (&a.when, &b.when) {
        (Some(x), Some(y)) => x.var == y.var && x.mask == y.mask && x.equals != y.equals,
        _ => false,
    }
}

fn validate_action(spec: &DisplaySpec, cmd: &DisplayCommand, action: &DisplayAction) -> Result<()> {
    let where_ = || format!("command 0x{:02X}", cmd.opcode);
    // An action is a ONE-KEY map. Zero keys is a `do:` entry that does nothing —
    // which is how a mistyped action name would read — and two is an entry whose
    // order of effect nothing states.
    match action.arms_set() {
        1 => {}
        0 => bail!(
            "{}: a `do:` entry sets no action. A mistyped action name parses as an empty \
             entry and would run silently.",
            where_()
        ),
        n => bail!(
            "{}: a `do:` entry sets {n} actions; one entry is one action, so the order \
             between them would be this engine's private business rather than the datasheet's.",
            where_()
        ),
    }

    let check_value = |v: &DisplayValue| -> Result<()> {
        let sources =
            v.value.is_some() as u8 + v.opcode_mask.is_some() as u8 + v.args.is_some() as u8;
        if sources != 1 {
            bail!(
                "{}: a value needs exactly one source (value / opcode_mask / args), found {sources}",
                where_()
            );
        }
        if let Some(idx) = &v.args {
            if idx.is_empty() {
                bail!("{}: args: [] selects no parameter byte", where_());
            }
            for i in idx {
                if *i >= cmd.args {
                    bail!(
                        "{}: args index {i} but the command takes {} parameter bytes",
                        where_(),
                        cmd.args
                    );
                }
            }
        }
        Ok(())
    };

    if let Some(w) = &action.set_window {
        check_axis(spec, w.axis).with_context(where_)?;
        check_value(&w.start)?;
        check_value(&w.end)?;
    }
    if let Some(c) = &action.set_cursor {
        check_axis(spec, c.axis).with_context(where_)?;
        check_value(&c.value)?;
    }
    if let Some(v) = &action.set_mode {
        check_value(v)?;
        if spec.addressing.modes.len() < 2 {
            bail!(
                "{}: set_mode on a panel that declares one addressing mode — the command could \
                 only ever select what is already selected",
                where_()
            );
        }
    }
    if let Some(sv) = &action.set_var {
        check_value(&sv.value)?;
        if !spec.vars.contains_key(&sv.name) {
            bail!("{}: set_var '{}' is not a declared var", where_(), sv.name);
        }
    }
    if let Some(rw) = &action.ram_write {
        match (&rw.plane, spec.ram.planes.is_empty()) {
            (Some(name), false) => {
                if !spec.ram.planes.iter().any(|p| p == name) {
                    bail!(
                        "{}: ram_write writes plane '{name}', which ram.planes does not declare",
                        where_()
                    );
                }
            }
            (None, false) => bail!(
                "{}: ram_write on a panel with planes {:?} names none. The plane is the only \
                 thing that tells this stream from the other one, so defaulting would paint \
                 one colour's image into the other's memory.",
                where_(),
                spec.ram.planes
            ),
            (Some(name), true) => bail!(
                "{}: ram_write writes plane '{name}' and ram.planes declares no planes",
                where_()
            ),
            (None, true) => {}
        }
    }
    if let Some(g) = &action.when {
        if g.arg >= cmd.args {
            bail!(
                "{}: `when` reads parameter {} but the command takes {} of them",
                where_(),
                g.arg,
                cmd.args
            );
        }
        if let Some(m) = g.mask {
            if m == 0 {
                bail!(
                    "{}: `when` masks the parameter with 0, so every value equals {} or none \
                     does — the guard reads as if it selected something",
                    where_(),
                    g.equals
                );
            }
            if g.equals & !m != 0 {
                bail!(
                    "{}: `when` requires 0x{:02X} through mask 0x{m:02X}, which no masked byte \
                     can equal",
                    where_(),
                    g.equals
                );
            }
        }
    }
    Ok(())
}

fn check_axis(spec: &DisplaySpec, axis: DisplayAxis) -> Result<()> {
    let ok = matches!(
        (axis, spec.ram.layout),
        (DisplayAxis::Col, _)
            | (DisplayAxis::Page, DisplayRamLayout::PageMajor)
            | (DisplayAxis::Row, DisplayRamLayout::RowMajor)
    );
    if !ok {
        bail!(
            "axis {axis:?} has no meaning on a {:?} frame memory",
            spec.ram.layout
        );
    }
    Ok(())
}

// ─── construction ──────────────────────────────────────────────────────────

impl GenericDisplay {
    pub fn from_descriptor(descriptor: &DeviceDescriptor) -> Result<Self> {
        validate_descriptor(descriptor)?;
        let spec = descriptor
            .behavior
            .display
            .as_ref()
            .context("declarative display is missing behavior.display")?
            .clone();
        Ok(Self::from_spec(spec))
    }

    pub fn from_yaml(yaml: &str) -> Result<Self> {
        Self::from_descriptor(&DeviceDescriptor::from_yaml(yaml)?)
    }

    fn from_spec(spec: DisplaySpec) -> Self {
        let width = spec.width as usize;
        let height = spec.height as usize;
        let pages = spec.ram.pages.unwrap_or(0) as usize;
        let unit_bytes = spec.pixel_format.write_unit_bytes();
        let mut by_opcode: Box<[Vec<u16>; 256]> = vec![Vec::new(); 256]
            .into_boxed_slice()
            .try_into()
            .expect("256 entries");
        for (i, cmd) in spec.commands.iter().enumerate() {
            for op in cmd.opcode..=cmd.opcode_end.unwrap_or(cmd.opcode) {
                by_opcode[op as usize].push(i as u16);
            }
        }
        let address = spec.default_address.unwrap_or(0);
        let vars = spec.vars.clone();
        let mode = spec.addressing.default;
        // A blank frame memory is the panel's ERASED value, not zero: an
        // e-paper powers on white (`0xFF`), an OLED powers on dark (`0x00`).
        let ram = vec![spec.ram.blank; spec.ram.bytes as usize];
        let screen = ram.clone();
        let col_units = width / spec.ram.units.col.pixels_per_step() as usize;
        let row_units = height / spec.ram.units.row.pixels_per_step() as usize;
        let plane_names = spec.ram.planes.clone();
        let plane_bytes = spec.ram.bytes as usize / plane_names.len().max(1);
        let mut dev = Self {
            by_opcode,
            width,
            height,
            pages,
            unit_bytes,
            col_units,
            row_units,
            plane_bytes,
            plane_names,
            address,
            cs_pin: String::new(),
            dc_pin: None,
            dc_source: None,
            dc_level: false,
            dc_wiring: DcWiring::Gpio,
            component_id: None,
            powered: true,
            display_on: spec.power_on.display_on,
            awake: spec.power_on.awake,
            inverted: spec.power_on.inverted,
            mode,
            vars,
            col: 0,
            row: 0,
            page: 0,
            col_start: 0,
            col_end: 0,
            row_start: 0,
            row_end: 0,
            page_start: 0,
            page_end: 0,
            ram,
            screen,
            refresh_generation: 0,
            framing: Framing::Idle,
            pending_cmd: 0,
            pending_idx: None,
            params: [0; 64],
            param_have: 0,
            param_want: 0,
            unit: [0; 4],
            unit_have: 0,
            plane: 0,
            ram_remaining: None,
            control: None,
            glass: None,
            elapsed_us: 0,
            spec,
        };
        dev.reset_window();
        dev
    }

    /// Power-on window: whatever the descriptor states, else the whole frame
    /// memory. Both panels ported first power on with the full extent, which is
    /// what their datasheets give as the CASET/RASET/column defaults.
    fn reset_window(&mut self) {
        self.col_start = 0;
        self.col_end = self
            .spec
            .window
            .col_end
            .unwrap_or_else(|| (self.col_units as u16).saturating_sub(1));
        self.row_start = 0;
        self.row_end = self
            .spec
            .window
            .row_end
            .unwrap_or_else(|| (self.row_units as u16).saturating_sub(1));
        self.page_start = 0;
        self.page_end = self
            .spec
            .window
            .page_end
            .unwrap_or_else(|| (self.pages as u16).saturating_sub(1));
        self.col = 0;
        self.row = 0;
        self.page = 0;
    }

    pub fn set_component_id(&mut self, id: impl Into<String>) {
        self.component_id = Some(id.into());
    }

    pub fn set_address(&mut self, address: u8) {
        self.address = address;
    }

    pub fn set_cs_pin(&mut self, pin: impl Into<String>) {
        self.cs_pin = pin.into();
    }

    pub fn set_dc_pin(&mut self, pin: impl Into<String>) {
        self.dc_pin = Some(pin.into());
    }

    /// Declare whether the module's supply is connected. Only ever called with
    /// `false`, and only when the compiled manifest says the supply pins are on
    /// no net.
    pub fn set_powered(&mut self, powered: bool) {
        self.powered = powered;
    }

    pub fn set_glass_window(&mut self, glass: GlassWindow) {
        self.glass = Some(glass);
    }

    // ── what the outside asks a panel ───────────────────────────────────

    /// The raw frame memory, in the controller's own layout.
    pub fn framebuffer(&self) -> &[u8] {
        &self.ram
    }

    /// Frame-memory bytes carrying at least one lit pixel (mono panels).
    pub fn ink_bytes(&self) -> usize {
        self.ram.iter().filter(|b| **b != 0).count()
    }

    /// Lit pixels across a 1-bpp frame memory.
    pub fn lit_pixels(&self) -> usize {
        self.ram.iter().map(|b| b.count_ones() as usize).sum()
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// DISPON, and a supply to hold it. An unpowered controller cannot latch
    /// the bus at all, so this is the state rather than a second opinion over it.
    pub fn display_on(&self) -> bool {
        self.powered && self.display_on
    }

    /// SLPOUT seen, and a supply. An unpowered panel is not asleep, it is off.
    pub fn awake(&self) -> bool {
        self.powered && self.awake
    }

    /// What a camera would see: DISPON, awake, and every `lit_requires` clause
    /// satisfied. A panel that got DISPON but never SLPOUT is dark on the bench
    /// however full frame memory is — and an emissive panel whose firmware
    /// never wrote a brightness is dark even with both.
    pub fn lit(&self) -> bool {
        self.powered
            && self.display_on
            && self.awake
            && self
                .spec
                .lit_requires
                .iter()
                .all(|r| self.vars.get(&r.var).copied().unwrap_or(0) >= r.min)
    }

    /// Which of the two real D/C wirings this placement uses.
    pub fn dc_wiring(&self) -> DcWiring {
        self.dc_wiring
    }

    pub fn set_dc_wiring(&mut self, wiring: DcWiring) {
        self.dc_wiring = wiring;
    }

    pub fn inverted(&self) -> bool {
        self.inverted
    }

    pub fn powered(&self) -> bool {
        self.powered
    }

    pub fn addressing_mode(&self) -> DisplayAddressingMode {
        self.mode
    }

    /// Read this panel's frame memory BY PLANE NAME, and what a refresh last
    /// put on the glass.
    ///
    /// ONE accessor, not one type per panel. The CLI and the browser used to
    /// reach an e-paper by casting to `Ssd1680Tricolor290` and then to
    /// `Uc8151dTricolor290`, so every panel that grew a second plane grew an
    /// arm in two more files. A panel with no planes reports none and the
    /// callers take their other branch.
    pub fn planes(&self) -> PlaneView<'_> {
        PlaneView { dev: self }
    }

    /// How many `refresh` actions have run. Zero forever on a panel whose
    /// frame memory IS the screen, which is why it is the e-paper's evidence
    /// and nobody else's.
    pub fn refresh_generation(&self) -> u32 {
        self.refresh_generation
    }

    // ── orientation ─────────────────────────────────────────────────────

    fn orientation_bits(&self) -> (bool, bool, bool) {
        match &self.spec.orientation {
            None => (false, false, false),
            Some(o) => {
                let v = self.vars.get(&o.var).copied().unwrap_or(0);
                (
                    v & u32::from(o.swap_bit) != 0,
                    v & u32::from(o.mirror_x_bit) != 0,
                    v & u32::from(o.mirror_y_bit) != 0,
                )
            }
        }
    }

    /// Addressable extent in the CURRENT orientation. A controller that
    /// exchanges the page and column address order changes what a legal column
    /// IS, so clamping to the portrait width would fold a correct landscape
    /// window back into portrait.
    fn addressable(&self) -> (u16, u16) {
        let (swap, _, _) = self.orientation_bits();
        if swap {
            (self.spec.height, self.spec.width)
        } else {
            (self.spec.width, self.spec.height)
        }
    }

    /// Addressable extent in COUNTER STEPS, in the current orientation. Equal
    /// to [`Self::addressable`] for every panel that addresses pixels; the
    /// SSD1680's column counter steps a byte, so its 128-pixel row is 16 steps
    /// and a window bound clamped to 127 would be sixteen rows off the end.
    fn addressable_units(&self) -> (u16, u16) {
        let (swap, _, _) = self.orientation_bits();
        let (w, h) = (self.col_units as u16, self.row_units as u16);
        if swap {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Map a logical (column, row) onto physical frame memory, which does not
    /// rotate.
    fn to_physical(&self, col: u16, row: u16) -> (usize, usize) {
        let (swap, mx, my) = self.orientation_bits();
        let (mut x, mut y) = if swap { (row, col) } else { (col, row) };
        if mx {
            x = (self.col_units as u16).saturating_sub(1).saturating_sub(x);
        }
        if my {
            y = (self.row_units as u16).saturating_sub(1).saturating_sub(y);
        }
        (x as usize, y as usize)
    }

    fn axis_max(&self, axis: DisplayAxis) -> u16 {
        let (aw, ah) = self.addressable_units();
        match axis {
            DisplayAxis::Col => aw.saturating_sub(1),
            DisplayAxis::Row => ah.saturating_sub(1),
            DisplayAxis::Page => (self.pages as u16).saturating_sub(1),
        }
    }

    // ── the wire ────────────────────────────────────────────────────────

    fn resolve(&self, v: &DisplayValue, axis: DisplayAxis) -> u32 {
        let mut out = if let Some(c) = v.value {
            c
        } else if let Some(m) = v.opcode_mask {
            u32::from((self.pending_cmd & m) >> v.opcode_shift)
        } else {
            let idx = v.args.as_deref().unwrap_or(&[]);
            idx.iter().fold(0u32, |acc, i| {
                (acc << 8) | u32::from(self.params[*i as usize])
            })
        };
        if let Some(m) = v.mask {
            out &= m;
        }
        if v.clamp_axis_max {
            out = out.min(u32::from(self.axis_max(axis)));
        }
        out
    }

    fn cursor_mut(&mut self, axis: DisplayAxis) -> &mut u16 {
        match axis {
            DisplayAxis::Col => &mut self.col,
            DisplayAxis::Row => &mut self.row,
            DisplayAxis::Page => &mut self.page,
        }
    }

    fn run_actions(&mut self, cmd_index: usize) {
        // The action list is cloned out of the spec because running an action
        // mutates `self`; the clone is per COMMAND, not per byte, and a command
        // table entry is a handful of small enums.
        let actions = self.spec.commands[cmd_index].actions.clone();
        for action in &actions {
            // A PARAMETER-GUARDED entry. The SSD1680's 0x22 is a sequence
            // selector: 0xF8 powers the booster on, 0x83 powers it off, and
            // every other value leaves it alone. The guard reads the parameter
            // bytes this command just collected.
            if let Some(g) = &action.when {
                let byte = self.params[g.arg as usize];
                if g.mask.map_or(byte, |m| byte & m) != g.equals {
                    continue;
                }
            }
            if let Some(w) = &action.set_window {
                let start = self.resolve(&w.start, w.axis) as u16;
                let end = self.resolve(&w.end, w.axis) as u16;
                match w.axis {
                    DisplayAxis::Col => {
                        self.col_start = start;
                        self.col_end = end;
                        self.col = start;
                    }
                    DisplayAxis::Row => {
                        self.row_start = start;
                        self.row_end = end;
                        self.row = start;
                    }
                    DisplayAxis::Page => {
                        self.page_start = start;
                        self.page_end = end;
                        self.page = start;
                    }
                }
            }
            if let Some(c) = &action.set_cursor {
                let v = self.resolve(&c.value, c.axis) as u16;
                let cur = self.cursor_mut(c.axis);
                *cur = match c.part {
                    DisplayCursorPart::All => v,
                    DisplayCursorPart::LowNibble => (*cur & 0xFFF0) | (v & 0x0F),
                    DisplayCursorPart::HighNibble => (*cur & 0xFF0F) | ((v & 0x0F) << 4),
                };
            }
            if let Some(v) = &action.set_mode {
                let n = self.resolve(v, DisplayAxis::Col);
                // An encoding the table does not cover selects the last declared
                // mode, which is what a controller with a two-bit field and three
                // modes does with the fourth value.
                let modes = &self.spec.addressing.modes;
                let idx = (n as usize).min(modes.len().saturating_sub(1));
                self.mode = modes[idx];
            }
            if let Some(sv) = &action.set_var {
                let n = self.resolve(&sv.value, DisplayAxis::Col);
                self.vars.insert(sv.name.clone(), n);
            }
            if let Some(rw) = &action.ram_write {
                if rw.reset_cursor {
                    self.col = self.col_start;
                    self.row = self.row_start;
                    self.page = self.page_start;
                }
                // WHICH plane this stream writes. Validation has already proved
                // the name is declared when the panel has planes and absent
                // when it has none, so an unknown name here cannot happen.
                self.plane = rw
                    .plane
                    .as_deref()
                    .and_then(|n| self.plane_names.iter().position(|p| p == n))
                    .unwrap_or(0);
                self.unit_have = 0;
                self.ram_remaining = (self.spec.ram.stream == DisplayRamStream::WindowCounted)
                    .then(|| self.window_units());
                self.framing = Framing::Ram;
            }
            if let Some(on) = action.display_on {
                self.display_on = on;
            }
            if let Some(a) = action.awake {
                self.awake = a;
            }
            if let Some(i) = action.invert {
                self.inverted = i;
            }
            if action.reset_control {
                self.display_on = self.spec.power_on.display_on;
                self.awake = self.spec.power_on.awake;
                self.inverted = self.spec.power_on.inverted;
                self.mode = self.spec.addressing.default;
                self.vars = self.spec.vars.clone();
                self.reset_window();
            }
            if action.clear_ram {
                self.ram.fill(self.spec.ram.blank);
            }
            // PUT FRAME MEMORY ON THE GLASS. Until this runs, an e-paper still
            // shows the previous image however full its RAM is, and
            // `refresh_generation` is the only thing that says so.
            if action.refresh {
                self.screen.copy_from_slice(&self.ram);
                self.refresh_generation = self.refresh_generation.wrapping_add(1);
            }
        }
    }

    /// Write units the current window holds — the stream length a
    /// `window_counted` `ram_write` opens.
    fn window_units(&self) -> u32 {
        let cols = u32::from(self.col_end.saturating_sub(self.col_start)) + 1;
        let secondary = if self.spec.ram.layout == DisplayRamLayout::PageMajor {
            u32::from(self.page_end.saturating_sub(self.page_start)) + 1
        } else {
            u32::from(self.row_end.saturating_sub(self.row_start)) + 1
        };
        cols * secondary
    }

    /// One command byte off the wire.
    fn command_byte(&mut self, byte: u8) {
        // Parameters of a command opened on the COMMAND stream (I²C control-byte
        // framing) arrive here, ahead of any opcode decode — the controller is
        // counting bytes, not looking at their values.
        if self.param_want > 0 && !self.args_on_data_stream() {
            self.take_param(byte);
            return;
        }
        self.pending_cmd = byte;
        self.param_have = 0;
        self.param_want = 0;
        self.params = [0; 64];
        let resolved = self.lookup(byte);
        self.pending_idx = resolved;
        let Some(idx) = resolved else {
            // An opcode the controller does not implement is consumed and
            // ignored. On a D/C-framed panel it also CLOSES whatever stream was
            // open, which is what silicon does and what stops an init sequence's
            // stray byte from landing in frame memory.
            if self.args_on_data_stream() {
                self.framing = Framing::Idle;
            }
            return;
        };
        let idx = idx as usize;
        let want = self.spec.commands[idx].args;
        if want > 0 {
            self.param_want = want;
            self.framing = Framing::Params;
            return;
        }
        self.framing = Framing::Idle;
        self.run_actions(idx);
    }

    /// Which command-table entry decodes `op` right now. The first entry that
    /// claims the opcode and whose `when` guard holds; validation has already
    /// proved at most one can.
    fn lookup(&self, op: u8) -> Option<u16> {
        self.by_opcode[op as usize].iter().copied().find(|i| {
            match &self.spec.commands[*i as usize].when {
                None => true,
                Some(w) => {
                    let v = self.vars.get(&w.var).copied().unwrap_or(0);
                    w.mask.map_or(v, |m| v & m) == w.equals
                }
            }
        })
    }

    fn take_param(&mut self, byte: u8) {
        let have = self.param_have as usize;
        if have < self.params.len() {
            self.params[have] = byte;
        }
        self.param_have += 1;
        if self.param_have < self.param_want {
            return;
        }
        self.param_want = 0;
        self.param_have = 0;
        self.framing = Framing::Idle;
        if let Some(idx) = self.pending_idx {
            self.run_actions(idx as usize);
        }
    }

    /// One data byte off the wire.
    fn data_byte(&mut self, byte: u8) {
        // `ram.stream: always` — the data stream IS frame memory, with no RAMWR
        // opcode to open it. True of the SSD1306 and the SH1107 (I²C control
        // byte) and equally of the PCD8544 (a D/C pad and no RAMWR at all),
        // which is why this reads the descriptor rather than the framing.
        if self.spec.ram.stream == DisplayRamStream::Always {
            self.ram_byte(byte);
            return;
        }
        match self.framing {
            Framing::Params => self.take_param(byte),
            Framing::Ram => self.ram_byte(byte),
            // A data byte with no command open is a stray on silicon too.
            Framing::Idle => {}
        }
    }

    /// True when command PARAMETERS ride the data line — the 4-wire SPI panel,
    /// whose D/C pad goes high for a command's arguments as much as for pixels.
    fn args_on_data_stream(&self) -> bool {
        self.spec.dc.source != DisplayDcSource::ControlByte
    }

    fn ram_byte(&mut self, byte: u8) {
        self.unit[self.unit_have] = byte;
        self.unit_have += 1;
        if self.unit_have < self.unit_bytes {
            return;
        }
        self.unit_have = 0;
        self.commit_unit();
        self.advance();
        // `window_counted`: the controller accepts exactly the window and then
        // the stream is shut. Running past it would wrap the counters back to
        // the window start and overwrite the rows just written.
        if let Some(left) = self.ram_remaining.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                self.ram_remaining = None;
                self.framing = Framing::Idle;
            }
        }
    }

    fn commit_unit(&mut self) {
        // Where this plane starts. Zero for a panel with no planes, which is
        // every panel but the tri-colour e-papers.
        let base = self.plane * self.plane_bytes;
        match self.spec.ram.layout {
            DisplayRamLayout::PageMajor => {
                let idx = base + self.page as usize * self.col_units + self.col as usize;
                if idx < base + self.plane_bytes && idx < self.ram.len() {
                    self.ram[idx] = self.unit[0];
                }
            }
            DisplayRamLayout::RowMajor => {
                let (x, y) = self.to_physical(self.col, self.row);
                if x < self.col_units && y < self.row_units {
                    let idx = base + (y * self.col_units + x) * self.unit_bytes;
                    self.ram[idx..idx + self.unit_bytes]
                        .copy_from_slice(&self.unit[..self.unit_bytes]);
                }
            }
        }
    }

    /// Advance the address counters. A completed write increments the primary
    /// axis; past its window end it returns to the window start and increments
    /// the secondary; past that end too, both return to their starts.
    fn advance(&mut self) {
        let page_major = self.spec.ram.layout == DisplayRamLayout::PageMajor;
        match self.mode {
            DisplayAddressingMode::Horizontal => {
                let (sec, sec_start, sec_end) = if page_major {
                    (self.page, self.page_start, self.page_end)
                } else {
                    (self.row, self.row_start, self.row_end)
                };
                let mut sec_next = sec;
                if self.col >= self.col_end {
                    self.col = self.col_start;
                    sec_next = if sec >= sec_end { sec_start } else { sec + 1 };
                } else {
                    self.col += 1;
                }
                if page_major {
                    self.page = sec_next;
                } else {
                    self.row = sec_next;
                }
            }
            DisplayAddressingMode::Vertical => {
                if self.page >= self.page_end {
                    self.page = self.page_start;
                    if self.col >= self.col_end {
                        self.col = self.col_start;
                    } else {
                        self.col += 1;
                    }
                } else {
                    self.page += 1;
                }
            }
            DisplayAddressingMode::Page => {
                // Column only, over the FRAME MEMORY's extent (not the
                // window's): page addressing ignores the column window and the
                // page never changes. What happens AT the last column is the
                // one thing the two paged OLEDs here disagree about, so it is
                // `addressing.page_wrap` rather than a house rule.
                if (self.col as usize) < self.col_units.saturating_sub(1) {
                    self.col += 1;
                } else if self.spec.addressing.page_wrap == DisplayPageWrap::Wrap {
                    self.col = 0;
                }
            }
        }
    }

    // ── the paint artifact ──────────────────────────────────────────────

    /// Extent of the artifact: the fixed glass if one was declared, else the
    /// whole frame memory in the current firmware orientation.
    fn logical_dimensions(&self) -> (usize, usize) {
        if let Some(g) = self.glass {
            return (g.cols as usize, g.rows as usize);
        }
        let (aw, ah) = self.addressable();
        (aw as usize, ah as usize)
    }

    /// The configured glass in fixed physical coordinates, or the whole frame
    /// memory in firmware coordinates when no glass was declared. The
    /// orientation has ALREADY mapped pixel writes into physical memory;
    /// applying it again to a glass window would move the crop.
    fn oriented_framebuffer(&self) -> Vec<u8> {
        if self.glass.is_none() && self.spec.orientation.is_none() {
            return self.ram.clone();
        }
        let (w, h) = self.logical_dimensions();
        let bpp = self.unit_bytes;
        let mut out = vec![0u8; w * h * bpp];
        let (cx, cy) = match self.glass {
            Some(g) => (g.col_offset as usize, g.row_offset as usize),
            None => (0, 0),
        };
        for row in 0..h {
            for col in 0..w {
                let (x, y) = if self.glass.is_some() {
                    (col + cx, row + cy)
                } else {
                    self.to_physical(col as u16, row as u16)
                };
                if x >= self.width || y >= self.height {
                    continue;
                }
                let src = (y * self.width + x) * bpp;
                let dst = (row * w + col) * bpp;
                out[dst..dst + bpp].copy_from_slice(&self.ram[src..src + bpp]);
            }
        }
        out
    }

    fn build_artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        let fb = self.oriented_framebuffer();
        let (w, h) = self.logical_dimensions();
        let mut meta = serde_json::Map::new();
        // These four describe the PAYLOAD, so they are always present: a
        // consumer cannot unpack the bytes without them.
        meta.insert("w".into(), serde_json::json!(w));
        meta.insert("h".into(), serde_json::json!(h));
        meta.insert(
            "format".into(),
            serde_json::json!(self.spec.artifact_format),
        );
        meta.insert(
            "generation".into(),
            serde_json::json!(crate::inspect::artifact_generation(&fb)),
        );

        // The dominant colour is counted in a BTreeMap, not a HashMap: a tie
        // between two colours must resolve the same way on every run and in
        // both engines, and `max_by_key` over an ordered iterator does that by
        // construction.
        let top = |i: usize| {
            let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
            for px in fb.chunks_exact(2) {
                let v = u16::from_be_bytes([px[0], px[1]]);
                if v != 0 {
                    *counts.entry(v).or_default() += 1;
                }
            }
            let top = counts.into_iter().max_by_key(|&(_, n)| n);
            match i {
                0 => top.map(|(v, _)| serde_json::json!(format!("0x{v:04X}"))),
                _ => top.map(|(_, n)| serde_json::json!(n)),
            }
            .unwrap_or(serde_json::Value::Null)
        };

        for field in &self.spec.artifact_meta {
            // A named plane's ink count, off frame memory or off the glass.
            if let Some((plane, of)) = field.plane() {
                let view = self.planes();
                let blank = view.blank();
                let bytes = match of {
                    DisplayPlaneOf::Ram => view.ram(plane),
                    DisplayPlaneOf::Screen => view.screen(plane),
                };
                let n = bytes.map_or(0, |b| b.iter().filter(|&&x| x != blank).count());
                meta.insert(field.key().to_string(), serde_json::json!(n));
                continue;
            }
            if let Some(var) = field.var() {
                let n = self.vars.get(var).copied().unwrap_or(0);
                let value = match field.format() {
                    DisplayMetaFormat::Raw => serde_json::json!(n),
                    DisplayMetaFormat::Hex8 => serde_json::json!(format!("0x{n:02X}")),
                    DisplayMetaFormat::Hex16 => serde_json::json!(format!("0x{n:04X}")),
                };
                meta.insert(field.key().to_string(), value);
                continue;
            }
            let Some(flag) = field.flag() else { continue };
            let value = match flag {
                DisplayMetaFlag::InkBytes => {
                    serde_json::json!(fb.iter().filter(|b| **b != 0).count())
                }
                DisplayMetaFlag::LitPixels => {
                    serde_json::json!(fb.iter().map(|b| b.count_ones() as usize).sum::<usize>())
                }
                DisplayMetaFlag::PaintedBytes => {
                    serde_json::json!(fb.iter().filter(|&&b| b != 0x00).count())
                }
                DisplayMetaFlag::TotalBytes => serde_json::json!(fb.len()),
                DisplayMetaFlag::TopColour => top(0),
                DisplayMetaFlag::TopColourPixels => top(1),
                DisplayMetaFlag::DisplayOn => serde_json::json!(self.display_on()),
                DisplayMetaFlag::Awake => serde_json::json!(self.awake()),
                DisplayMetaFlag::Lit => serde_json::json!(self.lit()),
                DisplayMetaFlag::Powered => serde_json::json!(self.powered),
                DisplayMetaFlag::Inverted => serde_json::json!(self.inverted),
                // The RAW sleep flag, not `!awake()`. See the enum's note: an
                // unpowered panel has never been woken, so it is asleep.
                DisplayMetaFlag::Asleep => serde_json::json!(!self.awake),
                DisplayMetaFlag::DcSource => serde_json::json!(self.dc_wiring.as_str()),
                DisplayMetaFlag::RefreshGeneration => serde_json::json!(self.refresh_generation),
                DisplayMetaFlag::PlaneBytes => serde_json::json!(self.plane_bytes),
            };
            meta.insert(field.key().to_string(), value);
        }

        vec![crate::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::Value::Object(meta),
            bytes: crate::inspect::artifact_bytes(&fb, opts),
        }]
    }

    /// The picture, for a snapshot: frame memory, THE GLASS, and the refresh
    /// counter. Control state is deliberately NOT carried — it is rebuilt by
    /// replaying the bus, and a snapshot that resumed a cursor would continue a
    /// half-written frame at a position the wire never sent.
    ///
    /// The glass and the counter are here because on an e-paper they are not
    /// derivable from frame memory: a resumed panel whose screen was rebuilt
    /// from RAM would claim to be showing a frame that was written and never
    /// activated, and `min_refresh_generation` would resolve against a zero
    /// that the run had already passed.
    fn snapshot_ram(&self) -> Vec<u8> {
        let snap = DisplaySnapshot {
            tag: DISPLAY_SNAPSHOT_TAG,
            version: DISPLAY_SNAPSHOT_VERSION,
            ram: self.ram.clone(),
            screen: self.screen.clone(),
            refresh_generation: self.refresh_generation,
        };
        bincode::serialize(&snap).expect("bincode serialize DisplaySnapshot")
    }

    fn restore_ram(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        let refuse = |why: String| crate::SimulationError::NotImplemented(why);
        let snap: DisplaySnapshot = bincode::deserialize(bytes).map_err(|e| {
            refuse(format!(
                "display snapshot: not a tagged panel snapshot ({e}). Snapshots taken before                  the panel carried its glass and refresh counter cannot be resumed — retake it."
            ))
        })?;
        if snap.tag != DISPLAY_SNAPSHOT_TAG {
            return Err(refuse(format!(
                "display snapshot: tag 0x{:08X} is not 0x{DISPLAY_SNAPSHOT_TAG:08X}. An                  untagged snapshot is a pre-versioning capture of raw frame memory — retake it.",
                snap.tag
            )));
        }
        if snap.version != DISPLAY_SNAPSHOT_VERSION {
            return Err(refuse(format!(
                "display snapshot: version {} is not {DISPLAY_SNAPSHOT_VERSION} — retake it.",
                snap.version
            )));
        }
        if snap.ram.len() != self.ram.len() || snap.screen.len() != self.screen.len() {
            return Err(refuse(format!(
                "display snapshot: {} frame-memory bytes and {} glass bytes for a panel that                  holds {} of each",
                snap.ram.len(),
                snap.screen.len(),
                self.ram.len()
            )));
        }
        self.ram.copy_from_slice(&snap.ram);
        self.screen.copy_from_slice(&snap.screen);
        self.refresh_generation = snap.refresh_generation;
        Ok(())
    }
}

/// Magic word every panel snapshot starts with — `"LWDS"`, LabWired display
/// snapshot. THE POINT IS REFUSAL: before this existed a runtime snapshot was
/// raw frame memory with no header, so a capture taken by an older build
/// restored silently into a panel that now also carries a glass and a refresh
/// counter, and the resumed run reported a picture nobody had activated. A
/// blank e-paper's first four bytes are `0xFFFFFFFF` and an OLED's are zero;
/// neither is this word.
const DISPLAY_SNAPSHOT_TAG: u32 = 0x4C57_4453;
/// Bumped whenever the fields below change shape. See the tag.
const DISPLAY_SNAPSHOT_VERSION: u16 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct DisplaySnapshot {
    tag: u32,
    version: u16,
    ram: Vec<u8>,
    screen: Vec<u8>,
    refresh_generation: u32,
}

// ─── I²C door (control-byte framing) ───────────────────────────────────────

impl I2cDevice for GenericDisplay {
    fn address(&self) -> u8 {
        self.address
    }

    /// Write-only over I²C. Every panel that frames with a control byte is.
    fn read(&mut self) -> u8 {
        0
    }

    /// A START (or repeated START) begins a new transaction, so the next byte
    /// is a control byte again.
    ///
    /// Resetting only in `stop()` would be wrong: a driver that issues several
    /// transfers and one trailing STOP — which is what the nRF52 TWIM does, one
    /// STARTTX per transfer — would keep the latch set across the whole burst,
    /// every transfer after the first would lose its leading control byte into
    /// the command decoder, and a `0x40` data prefix would silently become a
    /// command. The framebuffer behind it is then parsed as opcodes: a garbled
    /// panel, not a NACK. Real silicon re-reads the control byte after every
    /// START.
    fn start(&mut self) {
        self.control = None;
    }

    fn write(&mut self, data: u8) {
        if !self.powered {
            return;
        }
        let Some(control) = self.control else {
            self.control = Some(data);
            return;
        };
        if Some(control) == self.spec.dc.command_value {
            self.command_byte(data);
        } else {
            self.data_byte(data);
        }
    }

    fn stop(&mut self) {
        self.control = None;
    }

    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        self.build_artifacts(id, opts)
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    // Both halves, deliberately. A one-sided impl compiles and passes every
    // unit test while the downcast that reads the framebuffer from outside
    // silently gets None.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn advance_time_us(&mut self, us: u64) {
        self.elapsed_us = self.elapsed_us.saturating_add(us);
    }
}

// ─── SPI door (D/C-pin framing) ────────────────────────────────────────────

impl SpiDevice for GenericDisplay {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    /// What CS↓ does to a half-open stream is `cs_select:` in the descriptor,
    /// not a house rule: the ST7789 treats CS as the transaction boundary,
    /// while the ILI9341 lets a RAMWR pixel stream survive a deselect because a
    /// driver that chunks a large blit releases CS between bursts.
    fn cs_select(&mut self) {
        if self.spec.cs_select == DisplayCsSelect::KeepsStream {
            return;
        }
        self.framing = Framing::Idle;
        self.param_want = 0;
        self.param_have = 0;
        self.unit_have = 0;
        self.ram_remaining = None;
    }

    fn dc_pin(&self) -> Option<&str> {
        self.dc_pin.as_deref()
    }

    fn set_dc_level(&mut self, level: bool) {
        self.dc_level = level;
    }

    fn dc_source(&self) -> Option<(u64, u8)> {
        self.dc_source
    }

    fn set_dc_source(&mut self, odr_addr: u64, bit: u8) {
        self.dc_source = Some((odr_addr, bit));
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // THE ONE GATE THAT MAKES AN UNPOWERED PANEL BEHAVE LIKE ONE. Every
        // state change this model has arrives through here, so refusing the bus
        // keeps `display_on` / `awake` / `lit` / `painted_bytes` at their
        // power-on-dark values BY CONSTRUCTION rather than by masking them at
        // report time.
        if !self.powered {
            return 0;
        }
        // NO D/C LINE RESOLVED AT ATTACH. What a panel does then is its own
        // declared cheat, never a house rule — see `DisplayDc::unwired`, and
        // FIDELITY.md §E. A panel that leaves the key at its default reads the
        // latched level anyway, which is what every panel did before the key
        // existed.
        if self.dc_source.is_none() {
            match self.spec.dc.unwired {
                DisplayDcUnwired::Level => {}
                // CHEAT(INFER): nothing open ⇒ this is a command — real:
                // sample the D/C pad. Only terminates because
                // `window_counted` closes the pixel stream. FIDELITY.md §E.
                DisplayDcUnwired::Infer => {
                    if self.framing == Framing::Idle {
                        self.command_byte(mosi);
                    } else {
                        self.data_byte(mosi);
                    }
                    return 0;
                }
                // CHEAT(INFER): every byte is data — real: sample the D/C
                // pad. FIDELITY.md §E.
                DisplayDcUnwired::Data => {
                    self.data_byte(mosi);
                    return 0;
                }
            }
        }
        let command = self.dc_level == (self.spec.dc.command_level != 0);
        if command {
            self.command_byte(mosi);
        } else {
            self.data_byte(mosi);
        }
        0
    }

    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        self.build_artifacts(id, opts)
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn advance_time_us(&mut self, us: u64) {
        self.elapsed_us = self.elapsed_us.saturating_add(us);
    }

    /// A save/restore carries the PIXELS. See [`GenericDisplay::snapshot_ram`].
    fn runtime_snapshot(&self) -> Vec<u8> {
        self.snapshot_ram()
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        self.restore_ram(bytes)
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

/// A [`PeripheralKit`] backed by a declarative `display` descriptor.
pub struct DeclarativeDisplayKit {
    descriptor: DeviceDescriptor,
    metadata: &'static KitMetadata,
}

impl DeclarativeDisplayKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        validate_descriptor(&descriptor)?;
        let metadata = leak_metadata(&descriptor);
        Ok(Self {
            descriptor,
            metadata,
        })
    }

    fn spec(&self) -> &DisplaySpec {
        self.descriptor
            .behavior
            .display
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
        .display
        .as_ref()
        .expect("validated at construction");
    let meta = descriptor.metadata.as_ref();
    let label = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| descriptor.r#type.clone());
    let summary = meta
        .and_then(|m| m.summary.clone())
        .unwrap_or_else(|| "Declarative display.".to_string());
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
    let (transport, category) = match spec.dc.source {
        DisplayDcSource::ControlByte => (Transport::I2c, Category::I2c),
        DisplayDcSource::Pin | DisplayDcSource::HwDcx => (Transport::Spi, Category::Spi),
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

impl PeripheralKit for DeclarativeDisplayKit {
    fn metadata(&self) -> &'static KitMetadata {
        self.metadata
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let spec = self.spec();
        let mut dev = GenericDisplay::from_descriptor(&self.descriptor)?;
        match spec.dc.source {
            DisplayDcSource::ControlByte => {
                let address = ctx.i2c_address_or(spec.default_address.unwrap_or(0))?;
                dev.set_address(address);
                ctx.attach_i2c_device(Box::new(dev))
            }
            DisplayDcSource::Pin | DisplayDcSource::HwDcx => {
                dev.set_cs_pin(ctx.config_str("cs_pin").unwrap_or("").to_string());
                let dc_pin = ctx.config_str("dc_pin").map(|s| s.to_string());
                // `hw_dcx` is only ever read for a panel that declares the
                // source. Reading it on a `pin` panel would let a descriptor
                // that never modelled the controller-driven line silently accept
                // a board wired that way and then never frame a command.
                let hw_dcx = spec.dc.source == DisplayDcSource::HwDcx
                    && ctx.config_bool("hw_dcx") == Some(true);
                match (dc_pin, hw_dcx) {
                    (Some(_), true) => anyhow::bail!(
                        "{} '{}': `dc_pin` and `hw_dcx` are mutually exclusive. Either the \
                         firmware drives D/C on a GPIO or the controller drives it from \
                         PSEL.DCX -- on real hardware only one line is connected.",
                        self.descriptor.r#type,
                        ctx.device_id(),
                    ),
                    // A panel that declares an `unwired` fallback accepts a
                    // board with no D/C pad — see `DisplayDc::unwired`. That is
                    // the ESP32 e-paper lab, whose manifest wires CS and
                    // nothing else, and refusing it here would delete a lab
                    // that has run for a year.
                    (None, false) if spec.dc.unwired != DisplayDcUnwired::Level => {}
                    (None, false) => anyhow::bail!(
                        "{} '{}': no D/C source. {}This panel frames commands from the D/C line \
                         and has no infer-from-byte-values fallback: that inference decodes a \
                         parameter byte 0x2C as RAMWR and writes the remaining init bytes into \
                         the framebuffer as pixels, leaving a blank screen and a blameless \
                         firmware.",
                        self.descriptor.r#type,
                        ctx.device_id(),
                        if spec.dc.source == DisplayDcSource::HwDcx {
                            "Set `dc_pin` for a firmware-driven GPIO, or `hw_dcx: true` when the                              SPI controller drives D/C itself (nRF54L SPIM PSEL.DCX). "
                        } else {
                            "Set `dc_pin`. "
                        },
                    ),
                    (None, true) => {
                        dev.set_dc_wiring(DcWiring::ControllerDcx);
                    }
                    (Some(dc), false) => {
                        // Resolving the pin to its GPIO output register is the
                        // half that makes D/C real: the bus samples that
                        // register before each transfer. Declaring the pin
                        // without this leaves D/C stuck low, every byte frames
                        // as a command, and the panel renders blank with no
                        // error.
                        let (odr_addr, bit) = ctx.resolve_pin_odr(&dc).ok_or_else(|| {
                            anyhow::anyhow!(
                                "{} '{}': D/C pin '{}' does not resolve to a driveable GPIO \
                                 output.",
                                self.descriptor.r#type,
                                ctx.device_id(),
                                dc,
                            )
                        })?;
                        dev.set_dc_pin(dc);
                        SpiDevice::set_dc_source(&mut dev, odr_addr, bit);
                        dev.set_dc_wiring(DcWiring::Gpio);
                    }
                }

                if spec.supply_gated && ctx.config_bool("powered") == Some(false) {
                    dev.set_powered(false);
                }
                // BUSY, driven once to its idle level. An undriven line is what
                // left the ESP32 e-reader lab blank: GxEPD2 spins in
                // `_waitWhileBusy` to a 30 s timeout that never arrives at
                // simulated speed. The POLARITY is the descriptor's, because
                // the two e-papers here are opposites.
                if let Some(busy) = &spec.busy {
                    if let Some(pin) = ctx.config_str(&busy.config_key) {
                        let pin = pin.to_string();
                        ctx.drive_pin_input(&pin, busy.idle_level)?;
                    }
                }
                if spec.glass_crop {
                    apply_glass_crop(&self.descriptor.r#type, ctx, spec, &mut dev)?;
                }
                ctx.attach_spi_device(Box::new(dev))
            }
        }
    }
}

/// A crop is all-or-nothing: a half-declared window would report a strip at a
/// guessed offset, which looks like a working display showing the wrong part of
/// the image.
fn apply_glass_crop(
    device_type: &str,
    ctx: &mut AttachCtx<'_>,
    spec: &DisplaySpec,
    dev: &mut GenericDisplay,
) -> Result<()> {
    let col_offset = ctx.config_i64("col_offset");
    let row_offset = ctx.config_i64("row_offset");
    let cols = ctx.config_i64("cols");
    let rows = ctx.config_i64("rows");
    if col_offset.is_none() && row_offset.is_none() && cols.is_none() && rows.is_none() {
        return Ok(());
    }
    let (Some(co), Some(ro), Some(c), Some(r)) = (col_offset, row_offset, cols, rows) else {
        anyhow::bail!(
            "{device_type} '{}': a visible window needs all four of `col_offset`, `row_offset`, \
             `cols` and `rows`. Declaring some of them would crop the artifact at a guessed \
             offset and render a plausible wrong picture.",
            ctx.device_id(),
        );
    };
    if co as usize + c as usize > spec.width as usize
        || ro as usize + r as usize > spec.height as usize
    {
        anyhow::bail!(
            "{device_type} '{}': visible window {}x{} at ({}, {}) runs past the {}x{} frame \
             memory this controller has.",
            ctx.device_id(),
            c,
            r,
            co,
            ro,
            spec.width,
            spec.height,
        );
    }
    dev.set_glass_window(GlassWindow {
        col_offset: co as u16,
        row_offset: ro as u16,
        cols: c as u16,
        rows: r as u16,
    });
    Ok(())
}

// ─── Registry statics ──────────────────────────────────────────────────────

use std::sync::LazyLock;

impl PeripheralKit for LazyLock<DeclarativeDisplayKit> {
    fn metadata(&self) -> &'static KitMetadata {
        LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        LazyLock::force(self).attach(ctx)
    }
}

/// Build a declarative display straight from its embedded descriptor. Used by
/// the engine's own tests and by any caller that wants the model without going
/// through a manifest.
pub fn embedded(device_type: &str) -> Result<GenericDisplay> {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .with_context(|| format!("no embedded descriptor for '{device_type}'"))?;
    GenericDisplay::from_yaml(yaml)
}

macro_rules! display_kit {
    ($(#[$meta:meta])* $ident:ident, $type:literal) => {
        $(#[$meta])*
        pub static $ident: LazyLock<DeclarativeDisplayKit> = LazyLock::new(|| {
            DeclarativeDisplayKit::from_yaml(
                labwired_config::embedded_device_yaml($type)
                    .expect(concat!($type, " descriptor embedded")),
            )
            .expect(concat!($type, " is a valid declarative display descriptor"))
        });
    };
}

display_kit!(
    /// Solomon Systech SSD1306, 0.96″ 128×64 (`ssd1306.yaml`).
    SSD1306_KIT,
    "oled-ssd1306"
);
display_kit!(
    /// Solomon Systech SSD1306, 0.91″ 128×32 (`ssd1306_128x32.yaml`).
    SSD1306_128X32_KIT,
    "oled-ssd1306-128x32"
);
display_kit!(
    /// Sitronix ST7789V on the 1.9″ 170×320 IPS module (`st7789.yaml`).
    ST7789_KIT,
    "st7789-170x320"
);
display_kit!(
    /// Sino Wealth SH1107, 1.5″ 128×128 (`sh1107.yaml`).
    SH1107_KIT,
    "oled-sh1107"
);
display_kit!(
    /// Philips PCD8544 on the Nokia 5110 module, 84×48 (`pcd8544.yaml`).
    PCD8544_KIT,
    "pcd8544"
);
display_kit!(
    /// ILI Technology ILI9341, 240×320 RGB565 TFT (`ili9341.yaml`).
    ILI9341_KIT,
    "ili9341"
);
display_kit!(
    /// Raydium RM67162, 240×536 RGB565 AMOLED (`rm67162.yaml`).
    RM67162_KIT,
    "amoled-rm67162"
);
display_kit!(
    /// Solomon Systech SSD1680, 128×296 tri-colour e-paper
    /// (`ssd1680_tricolor_290.yaml`).
    SSD1680_TRICOLOR_290_KIT,
    "ssd1680_tricolor_290"
);
display_kit!(
    /// UltraChip UC8151D, 128×296 tri-colour e-paper
    /// (`uc8151d_tricolor_290.yaml`).
    UC8151D_TRICOLOR_290_KIT,
    "uc8151d_tricolor_290"
);

/// The SSD1306 128×64 model, built from its embedded descriptor. The shape the
/// in-crate tests used to get from `Ssd1306::new`.
pub fn ssd1306(address: u8) -> GenericDisplay {
    let mut dev = embedded("oled-ssd1306").expect("oled-ssd1306 descriptor builds");
    dev.set_address(address);
    dev
}

/// The SSD1306 128×32 model, built from its embedded descriptor.
pub fn ssd1306_128x32(address: u8) -> GenericDisplay {
    let mut dev = embedded("oled-ssd1306-128x32").expect("oled-ssd1306-128x32 descriptor builds");
    dev.set_address(address);
    dev
}

/// The SH1107 model, built from its embedded descriptor. The shape the
/// in-crate tests used to get from `Sh1107::new`.
pub fn sh1107(address: u8) -> GenericDisplay {
    let mut dev = embedded("oled-sh1107").expect("oled-sh1107 descriptor builds");
    dev.set_address(address);
    dev
}

/// The PCD8544 model with its two pins wired, for tests that drive the wire
/// directly rather than through a manifest. The shape the in-crate tests used
/// to get from `Pcd8544::new`.
pub fn pcd8544(cs_pin: &str, dc_pin: &str) -> GenericDisplay {
    let mut dev = embedded("pcd8544").expect("pcd8544 descriptor builds");
    dev.set_cs_pin(cs_pin);
    dev.set_dc_pin(dc_pin);
    dev
}

/// The ILI9341 model with its two pins wired, for tests that drive the wire
/// directly rather than through a manifest.
pub fn ili9341(cs_pin: &str, dc_pin: &str) -> GenericDisplay {
    let mut dev = embedded("ili9341").expect("ili9341 descriptor builds");
    dev.set_cs_pin(cs_pin);
    dev.set_dc_pin(dc_pin);
    dev
}

/// The RM67162 model driven by the SPI controller's own DCX line, which is how
/// `examples/nrf54lm20a-snake` wires it. The shape the in-crate tests used to
/// get from `Rm67162::with_controller_dc`.
pub fn rm67162_hw_dcx(cs_pin: &str) -> GenericDisplay {
    let mut dev = embedded("amoled-rm67162").expect("amoled-rm67162 descriptor builds");
    dev.set_cs_pin(cs_pin);
    dev.set_dc_wiring(DcWiring::ControllerDcx);
    dev
}

/// The RM67162 model with a firmware-driven D/C GPIO.
pub fn rm67162_gpio_dc(cs_pin: &str, dc_pin: &str) -> GenericDisplay {
    let mut dev = embedded("amoled-rm67162").expect("amoled-rm67162 descriptor builds");
    dev.set_cs_pin(cs_pin);
    dev.set_dc_pin(dc_pin);
    dev.set_dc_wiring(DcWiring::Gpio);
    dev
}

/// The SSD1680 tri-colour e-paper, built from its embedded descriptor. The
/// shape the tests used to get from `Ssd1680Tricolor290::new`.
pub fn ssd1680_tricolor_290(cs_pin: &str) -> GenericDisplay {
    let mut dev = embedded("ssd1680_tricolor_290").expect("ssd1680_tricolor_290 descriptor builds");
    dev.set_cs_pin(cs_pin);
    dev
}

/// The UC8151D tri-colour e-paper, built from its embedded descriptor.
pub fn uc8151d_tricolor_290(cs_pin: &str) -> GenericDisplay {
    let mut dev = embedded("uc8151d_tricolor_290").expect("uc8151d_tricolor_290 descriptor builds");
    dev.set_cs_pin(cs_pin);
    dev
}

/// The ST7789 model with its two pins wired, for tests that drive the wire
/// directly rather than through a manifest.
pub fn st7789(cs_pin: &str, dc_pin: &str) -> GenericDisplay {
    let mut dev = embedded("st7789-170x320").expect("st7789-170x320 descriptor builds");
    dev.set_cs_pin(cs_pin);
    dev.set_dc_pin(dc_pin);
    dev
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every display descriptor this engine ships must LOAD. A descriptor that
    /// only fails when a canvas happens to place it is a lab that breaks in the
    /// browser for the person who placed it.
    const SHIPPED: &[&str] = &[
        "oled-ssd1306",
        "oled-ssd1306-128x32",
        "st7789-170x320",
        "oled-sh1107",
        "pcd8544",
        "ili9341",
        "amoled-rm67162",
    ];

    #[test]
    fn every_shipped_display_descriptor_loads() {
        for t in SHIPPED {
            embedded(t).unwrap_or_else(|e| panic!("{t}: {e}"));
        }
    }

    #[test]
    fn every_shipped_display_registers_a_kit_with_the_right_transport() {
        for t in SHIPPED {
            let kit = DeclarativeDisplayKit::from_yaml(
                labwired_config::embedded_device_yaml(t).expect("embedded"),
            )
            .unwrap_or_else(|e| panic!("{t}: {e}"));
            let m = kit.metadata();
            assert_eq!(m.device_type, *t);
            assert!(!m.config_keys.is_empty(), "{t}: no config keys");
        }
    }

    /// A descriptor edited so its stated RAM size no longer matches its geometry
    /// must be REFUSED at load. Without this the counters walk off the end of a
    /// short buffer and every write past it is silently dropped — a panel that
    /// paints its top half.
    #[test]
    fn a_ram_size_that_contradicts_the_geometry_is_refused() {
        let yaml = labwired_config::embedded_device_yaml("oled-ssd1306").expect("embedded");
        let broken = yaml.replace("bytes: 1024", "bytes: 512");
        assert_ne!(
            broken, yaml,
            "the sabotage did not apply — this test measures nothing"
        );
        let err = GenericDisplay::from_yaml(&broken).expect_err("short RAM must be refused");
        assert!(
            format!("{err:#}").contains("ram.bytes"),
            "unexpected error: {err:#}"
        );
    }

    /// The negative control for the parity suite: a one-character change to the
    /// SSD1306 command table must change what the panel paints. If this passes
    /// while `display_migration_parity` still passes, the parity suite is not
    /// reading the descriptor.
    #[test]
    fn moving_a_mask_in_the_command_table_moves_the_pixels() {
        let yaml = labwired_config::embedded_device_yaml("oled-ssd1306").expect("embedded");
        // SETPAGESTART takes the page from the opcode's low three bits. Mask it
        // to one bit instead and page 3 becomes page 1.
        let sabotaged = yaml.replace(
            "- set_cursor: { axis: page, value: { opcode_mask: 0x07 } }",
            "- set_cursor: { axis: page, value: { opcode_mask: 0x01 } }",
        );
        assert_ne!(sabotaged, yaml, "the sabotage did not apply");

        let paint = |desc: &str| {
            let mut d = GenericDisplay::from_yaml(desc).expect("descriptor builds");
            d.start();
            d.write(0x00);
            d.write(0xB3); // page 3
            d.start();
            d.write(0x40);
            d.write(0xFF);
            d.stop();
            d.framebuffer()
                .iter()
                .position(|&b| b != 0)
                .expect("something was painted")
        };
        assert_eq!(paint(yaml), 3 * 128, "page 3 is byte 384");
        assert_eq!(
            paint(&sabotaged),
            128,
            "page 1: the sabotaged mask must move it"
        );
    }

    /// The negative control for `addressing.page_wrap`. Flipping the SH1107's
    /// declared `wrap` to `clamp` must pile the overflow bytes on the last
    /// column instead of wrapping them to column 0. If this passes while the
    /// SH1107 parity tests still pass, the key is not wired to the counter.
    #[test]
    fn page_wrap_clamp_and_wrap_paint_different_columns() {
        let yaml = labwired_config::embedded_device_yaml("oled-sh1107").expect("embedded");
        let clamped = yaml.replace("page_wrap: wrap", "page_wrap: clamp");
        assert_ne!(clamped, yaml, "the sabotage did not apply");

        let paint = |desc: &str| -> Vec<u8> {
            let mut d = GenericDisplay::from_yaml(desc).expect("descriptor builds");
            d.start();
            d.write(0x00);
            d.write(0x20); // page addressing
            d.write(0xB0); // page 0
            d.write(0x0E); // column low nibble  → 0x7E
            d.write(0x17); // column high nibble
            d.start();
            d.write(0x40);
            for b in [0x11u8, 0x22, 0x33] {
                d.write(b);
            }
            d.stop();
            d.framebuffer()[..128].to_vec()
        };
        let wrapped = paint(yaml);
        assert_eq!(
            [wrapped[126], wrapped[127], wrapped[0]],
            [0x11, 0x22, 0x33],
            "wrap: the third byte returns to column 0"
        );
        let held = paint(&clamped);
        assert_eq!(
            [held[126], held[127], held[0]],
            [0x11, 0x33, 0x00],
            "clamp: the third byte overwrites the last column"
        );
    }

    /// The negative control for `when:`. Moving the PCD8544's SET X entry out
    /// of the basic instruction set — so both readings of `0x80|n` become
    /// unguarded — must be REFUSED at load. Before the guard existed, the
    /// stock init's `0xBF` was read as "column 63" and the first frame landed
    /// 63 columns across.
    #[test]
    fn two_unguarded_entries_claiming_one_opcode_are_refused() {
        let yaml = labwired_config::embedded_device_yaml("pcd8544").expect("embedded");
        let broken = yaml.replace(
            "name: SETXADDR, when: { var: h, equals: 0 }",
            "name: SETXADDR",
        );
        assert_ne!(broken, yaml, "the sabotage did not apply");
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(format!("{err:#}").contains("claimed twice"), "got: {err:#}");
    }

    /// And the guard has to be WIRED, not merely declared. Move SET X into the
    /// EXTENDED set — the reading a flat table would have had to pick — and the
    /// stock init's `0xBF` becomes "column 63", so the first frame lands 63
    /// columns across. Two edits, because the shipped table would otherwise
    /// refuse the duplicate claim on `0x80..0xFF`.
    #[test]
    fn reading_set_x_in_the_wrong_instruction_set_moves_the_pixels() {
        let yaml = labwired_config::embedded_device_yaml("pcd8544").expect("embedded");
        let sabotaged = yaml
            .replace(
                "      - { opcode: 0x80, opcode_end: 0xFF, name: SETVOP,       when: { var: h, equals: 1 } }\n",
                "",
            )
            .replace("name: SETXADDR, when: { var: h, equals: 0 }", "name: SETXADDR, when: { var: h, equals: 1 }");
        assert_ne!(sabotaged, yaml, "the sabotage did not apply");

        let paint = |desc: &str| -> usize {
            let mut d = GenericDisplay::from_yaml(desc).expect("descriptor builds");
            for (dc, b) in [
                (false, 0x21u8), // extended instruction set
                (false, 0xBF),   // set Vop — NOT a column move
                (false, 0x20),   // basic instruction set
                (false, 0x0C),   // display normal
                (true, 0x5A),    // one pixel byte
            ] {
                d.set_dc_level(dc);
                d.transfer(b);
            }
            d.framebuffer()
                .iter()
                .position(|&b| b != 0)
                .expect("something was painted")
        };
        assert_eq!(
            paint(yaml),
            0,
            "the guarded table lands the byte at column 0"
        );
        assert_eq!(
            paint(&sabotaged),
            0x3F,
            "SET X read in the extended set decodes 0xBF as column 63"
        );
    }

    /// The negative control for `cs_select`. The ILI9341 declares
    /// `keeps_stream`; flipping it to the ST7789's `closes_stream` must DROP
    /// the pixels a chunked blit sends after releasing CS.
    #[test]
    fn cs_select_closes_stream_drops_a_resumed_blit() {
        let yaml = labwired_config::embedded_device_yaml("ili9341").expect("embedded");
        let closing = yaml.replace("cs_select: keeps_stream", "cs_select: closes_stream");
        assert_ne!(closing, yaml, "the sabotage did not apply");

        let paint = |desc: &str| -> usize {
            let mut d = GenericDisplay::from_yaml(desc).expect("descriptor builds");
            let cmd = |d: &mut GenericDisplay, op: u8, args: &[u8]| {
                SpiDevice::set_dc_level(d, false);
                SpiDevice::transfer(d, op);
                SpiDevice::set_dc_level(d, true);
                for a in args {
                    SpiDevice::transfer(d, *a);
                }
            };
            SpiDevice::cs_select(&mut d);
            cmd(&mut d, 0x2A, &[0x00, 0x00, 0x00, 0x03]);
            cmd(&mut d, 0x2B, &[0x00, 0x00, 0x00, 0x00]);
            cmd(&mut d, 0x2C, &[0x11, 0x11, 0x22, 0x22]);
            SpiDevice::cs_release(&mut d);
            SpiDevice::cs_select(&mut d);
            SpiDevice::set_dc_level(&mut d, true);
            for b in [0x33u8, 0x33, 0x44, 0x44] {
                SpiDevice::transfer(&mut d, b);
            }
            d.framebuffer().iter().filter(|&&b| b != 0).count()
        };
        assert_eq!(paint(yaml), 8, "keeps_stream: all four pixels land");
        assert_eq!(
            paint(&closing),
            4,
            "closes_stream: the resumed half of the blit is dropped"
        );
    }

    // ── `lit_requires`, `hw_dcx` and var meta: the RM67162's keys ──────────

    /// THE NEGATIVE CONTROL FOR `lit_requires`. Deleting the clause from the
    /// RM67162 descriptor must light a panel whose firmware never wrote a
    /// brightness. If this passes with the clause gone, the key is not wired to
    /// `lit` and `rm67162_dispon_without_brightness_is_not_lit` is proving
    /// nothing.
    #[test]
    fn deleting_lit_requires_lights_a_panel_at_zero_brightness() {
        let yaml = labwired_config::embedded_device_yaml("amoled-rm67162").expect("embedded");
        let without = yaml.replace("    lit_requires: [{ var: brightness, min: 1 }]\n", "");
        assert_ne!(without, yaml, "the sabotage did not apply");

        let lit = |desc: &str| -> bool {
            let mut d = GenericDisplay::from_yaml(desc).expect("descriptor builds");
            let cmd = |d: &mut GenericDisplay, op: u8| {
                SpiDevice::set_dc_level(d, false);
                SpiDevice::transfer(d, op);
                SpiDevice::set_dc_level(d, true);
            };
            SpiDevice::cs_select(&mut d);
            cmd(&mut d, 0x11); // SLPOUT
            cmd(&mut d, 0x29); // DISPON — and no WRDISBV anywhere
            d.lit()
        };
        assert!(
            !lit(yaml),
            "with the clause, brightness 0 is dark — the AMOLED assertion"
        );
        assert!(
            lit(&without),
            "without the clause the same firmware reads lit, which is the bug \
             the key exists to prevent"
        );
    }

    /// A `lit_requires` clause naming an undeclared var would read a cell
    /// nothing can write, so the panel would be dark for every firmware.
    #[test]
    fn lit_requires_on_an_undeclared_var_is_refused() {
        let yaml = labwired_config::embedded_device_yaml("amoled-rm67162").expect("embedded");
        let broken = yaml.replace("var: brightness, min: 1", "var: backlight, min: 1");
        assert_ne!(broken, yaml, "the sabotage did not apply");
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(format!("{err:#}").contains("backlight"), "got: {err:#}");
    }

    /// `min: 0` is satisfied by every value, so the clause gates nothing while
    /// reading as if it did.
    #[test]
    fn a_lit_requires_min_of_zero_is_refused() {
        let yaml = labwired_config::embedded_device_yaml("amoled-rm67162").expect("embedded");
        let broken = yaml.replace("var: brightness, min: 1", "var: brightness, min: 0");
        assert_ne!(broken, yaml, "the sabotage did not apply");
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(format!("{err:#}").contains("gates nothing"), "got: {err:#}");
    }

    /// An `artifact_meta` var entry must name a declared var, or the key would
    /// report a constant forever.
    #[test]
    fn an_artifact_meta_var_that_is_not_declared_is_refused() {
        let yaml = labwired_config::embedded_device_yaml("amoled-rm67162").expect("embedded");
        let broken = yaml.replace(
            "- { var: colmod, format: hex8 }",
            "- { var: gamma, format: hex8 }",
        );
        assert_ne!(broken, yaml, "the sabotage did not apply");
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(format!("{err:#}").contains("gamma"), "got: {err:#}");
    }

    /// `format:` is a published contract, not cosmetics: a consumer that parsed
    /// `"0x55"` reads `85` if the key silently becomes a number. The sabotage
    /// must change what the artifact carries.
    #[test]
    fn a_var_meta_format_changes_the_published_value() {
        let yaml = labwired_config::embedded_device_yaml("amoled-rm67162").expect("embedded");
        let raw = yaml.replace("- { var: colmod, format: hex8 }", "- { var: colmod }");
        assert_ne!(raw, yaml, "the sabotage did not apply");
        let colmod = |desc: &str| -> serde_json::Value {
            let d = GenericDisplay::from_yaml(desc).expect("descriptor builds");
            SpiDevice::artifacts(&d, "amoled", &crate::inspect::InspectOpts::default())[0].meta
                ["colmod"]
                .clone()
        };
        assert_eq!(colmod(yaml), serde_json::json!("0x55"));
        assert_eq!(colmod(&raw), serde_json::json!(0x55));
    }

    /// `dc_source` names WHICH of two wirings drives D/C. On a panel whose
    /// descriptor admits only one, the key would be a constant dressed as a
    /// measurement.
    #[test]
    fn dc_source_meta_on_a_single_wiring_panel_is_refused() {
        let yaml = labwired_config::embedded_device_yaml("ili9341").expect("embedded");
        let broken = yaml.replace(
            "artifact_meta: [display_on,",
            "artifact_meta: [dc_source, display_on,",
        );
        assert_ne!(broken, yaml, "the sabotage did not apply");
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(format!("{err:#}").contains("dc_source"), "got: {err:#}");
    }

    /// `ram.stream` and the command table must agree. `always` means every data
    /// byte is frame memory; a `ram_write` in the table says otherwise, and one
    /// of the two would silently win.
    #[test]
    fn a_ram_stream_that_contradicts_the_command_table_is_refused() {
        let yaml = labwired_config::embedded_device_yaml("st7789-170x320").expect("embedded");
        let broken = yaml.replace("stream: command", "stream: always");
        assert_ne!(broken, yaml, "the sabotage did not apply");
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(format!("{err:#}").contains("ram_write"), "got: {err:#}");
    }

    /// The other direction: a panel whose data line only becomes frame memory
    /// after a RAMWR, with no RAMWR anywhere in its table, could never paint.
    #[test]
    fn a_command_stream_with_no_ram_write_is_refused() {
        let broken = ssd1306_yaml_with(("stream: always", "stream: command"));
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(
            format!("{err:#}").contains("no data byte could ever reach frame memory"),
            "got: {err:#}"
        );
    }

    /// An artifact that carries pixels and nothing else cannot explain a dark
    /// frame, so an empty `artifact_meta` is a load error rather than a quiet
    /// four-key artifact.
    #[test]
    fn an_empty_artifact_meta_is_refused() {
        let broken = ssd1306_yaml_with((
            "artifact_meta: [ink_bytes, lit_pixels]",
            "artifact_meta: []",
        ));
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(
            format!("{err:#}").contains("artifact_meta is empty"),
            "got: {err:#}"
        );
    }

    /// A flag that cannot be computed for this pixel format is refused rather
    /// than published as a plausible number: `lit_pixels` over RGB565 counts
    /// set bits in colour values.
    #[test]
    fn an_artifact_flag_the_pixel_format_cannot_carry_is_refused() {
        let yaml = labwired_config::embedded_device_yaml("st7789-170x320").expect("embedded");
        let broken = yaml.replace("[display_on, lit, awake", "[lit_pixels, lit, awake");
        assert_ne!(broken, yaml, "the sabotage did not apply");
        let err = GenericDisplay::from_yaml(&broken).expect_err("must be refused");
        assert!(format!("{err:#}").contains("1 bpp ink"), "got: {err:#}");
    }

    // ── the e-paper keys: planes, refresh, byte units, the unwired cheat ───
    //
    // Each of these SABOTAGES the shipped descriptor and asserts what the
    // sabotage did. A validation message alone would only prove the engine can
    // print; these prove the key is load-bearing.

    fn epd_yaml_with(device: &str, replacement: (&str, &str)) -> String {
        let yaml = labwired_config::embedded_device_yaml(device).expect("embedded");
        let out = yaml.replace(replacement.0, replacement.1);
        assert_ne!(out, yaml, "the edit '{}' did not apply", replacement.0);
        out
    }

    /// Drive an SSD1680-shaped script: window the whole glass, open a plane
    /// stream, write eight bytes of ink.
    fn epd_write_black(dev: &mut GenericDisplay) {
        epd_write_black_value(dev, 0x00);
    }

    /// The same script, writing a chosen byte.
    fn epd_write_black_value(dev: &mut GenericDisplay, value: u8) {
        SpiDevice::set_dc_source(dev, 0x4000_0000, 0);
        for (op, params) in [
            (0x44u8, &[0x00u8, 0x0F][..]),
            (0x45, &[0x00, 0x00, 0x27, 0x01][..]),
            (0x24, &[][..]),
        ] {
            dev.set_dc_level(false);
            dev.transfer(op);
            dev.set_dc_level(true);
            for p in params {
                dev.transfer(*p);
            }
        }
        dev.set_dc_level(true);
        for _ in 0..8 {
            dev.transfer(value);
        }
    }

    fn epd_planes(dev: &GenericDisplay) -> (usize, usize) {
        let v = dev.planes();
        (
            v.ink_bytes("black").expect("black plane"),
            v.ink_bytes("red").expect("red plane"),
        )
    }

    /// TRANSPOSING THE TWO PLANES PAINTS THE OTHER COLOUR. 0x24 is the black
    /// RAM and 0x26 the red one; a descriptor that swapped them would report a
    /// red image for a black one and pass every count-only assertion.
    #[test]
    fn swapping_the_two_epaper_planes_paints_the_other_colour() {
        let mut stock = embedded("ssd1680_tricolor_290").expect("embedded");
        epd_write_black(&mut stock);
        assert_eq!(epd_planes(&stock), (8, 0), "0x24 writes the BLACK plane");

        let sabotaged = epd_yaml_with(
            "ssd1680_tricolor_290",
            (
                "{ opcode: 0x24, name: WRITE_RAM_BLACK, do: [ { ram_write: { reset_cursor: true, plane: black } } ] }",
                "{ opcode: 0x24, name: WRITE_RAM_BLACK, do: [ { ram_write: { reset_cursor: true, plane: red } } ] }",
            ),
        );
        let mut moved = GenericDisplay::from_yaml(&sabotaged).expect("still a valid descriptor");
        epd_write_black(&mut moved);
        assert_eq!(
            epd_planes(&moved),
            (0, 8),
            "the same 0x24 stream now lands in the RED plane",
        );
    }

    /// `ram.blank` IS THE ERASED BYTE, and on this panel a SET bit is NO ink.
    /// Declaring 0x00 inverts every ink count: `clearScreen(0xFF)` — the white
    /// frame GxEPD2 sends first — is then reported as a fully inked plane.
    #[test]
    fn an_epaper_blank_byte_of_zero_inverts_every_ink_count() {
        let mut stock = embedded("ssd1680_tricolor_290").expect("embedded");
        assert_eq!(epd_planes(&stock), (0, 0), "a fresh panel carries no ink");
        epd_write_black_value(&mut stock, 0xFF);
        assert_eq!(epd_planes(&stock), (0, 0), "0xFF is white: still no ink");
        epd_write_black_value(&mut stock, 0x00);
        assert_eq!(epd_planes(&stock), (8, 0), "0x00 is ink");

        let sabotaged = epd_yaml_with("ssd1680_tricolor_290", ("blank: 0xFF", "blank: 0x00"));
        let mut wrong = GenericDisplay::from_yaml(&sabotaged).expect("still a valid descriptor");
        epd_write_black_value(&mut wrong, 0xFF);
        assert_eq!(
            epd_planes(&wrong),
            (8, 0),
            "a white frame now reads as eight inked bytes",
        );
        epd_write_black_value(&mut wrong, 0x00);
        assert_eq!(
            epd_planes(&wrong),
            (0, 0),
            "and an inked frame reads as blank"
        );
    }

    /// THE X WINDOW IS IN BYTES. Reading it in pixels does not merely move the
    /// picture — it contradicts the stated RAM size, which is how the
    /// descriptor catches it at load rather than at paint.
    #[test]
    fn reading_the_epaper_x_window_in_pixels_is_refused() {
        let sabotaged = epd_yaml_with(
            "ssd1680_tricolor_290",
            (
                "units: { col: bytes, row: pixels }",
                "units: { col: pixels, row: pixels }",
            ),
        );
        let err = GenericDisplay::from_yaml(&sabotaged).expect_err("pixel units must be refused");
        assert!(
            format!("{err:#}").contains("ram.bytes"),
            "unexpected error: {err:#}"
        );
    }

    /// The SSD1680's unwired-D/C inference only TERMINATES because the stream
    /// is window-counted. Pairing `infer` with an uncounted stream is refused,
    /// naming the trap.
    #[test]
    fn inferring_framing_on_an_uncounted_stream_is_refused() {
        let sabotaged = epd_yaml_with(
            "ssd1680_tricolor_290",
            ("stream: window_counted", "stream: command"),
        );
        let err = GenericDisplay::from_yaml(&sabotaged).expect_err("infer needs a counted stream");
        let msg = format!("{err:#}");
        assert!(msg.contains("infer"), "unexpected error: {msg}");
    }

    /// A `ram_write` that names no plane on a two-plane panel is refused: a
    /// default would paint one colour's image into the other's memory.
    #[test]
    fn a_ram_write_with_no_plane_on_a_multi_plane_panel_is_refused() {
        let sabotaged = epd_yaml_with(
            "uc8151d_tricolor_290",
            (
                "ram_write: { reset_cursor: true, plane: black }",
                "ram_write: { reset_cursor: true }",
            ),
        );
        let err = GenericDisplay::from_yaml(&sabotaged).expect_err("a nameless plane is refused");
        assert!(
            format!("{err:#}").contains("names none"),
            "unexpected error: {err:#}"
        );
    }

    /// `refresh` and `refresh_generation` must both exist or neither: an
    /// artifact key that can only ever read 0 is worse than no key.
    #[test]
    fn a_refresh_action_without_its_counter_is_refused_and_so_is_the_counter_alone() {
        let no_counter = epd_yaml_with(
            "ssd1680_tricolor_290",
            (
                "      - refresh_generation
",
                "",
            ),
        );
        let err = GenericDisplay::from_yaml(&no_counter).expect_err("refresh needs its counter");
        assert!(
            format!("{err:#}").contains("refresh_generation"),
            "unexpected error: {err:#}"
        );

        let no_action = epd_yaml_with(
            "ssd1680_tricolor_290",
            (
                "{ opcode: 0x20, name: MASTER_ACTIVATION, do: [ { refresh: true } ] }",
                "{ opcode: 0x20, name: MASTER_ACTIVATION }",
            ),
        );
        let err = GenericDisplay::from_yaml(&no_action).expect_err("the counter needs an action");
        assert!(
            format!("{err:#}").contains("report 0 for every firmware"),
            "unexpected error: {err:#}"
        );
    }

    /// MOVING THE 0x22 SEQUENCE SELECTOR CHANGES WHAT POWERS THE BOOSTER.
    /// The guard reads a parameter byte; pointing it at the wrong value leaves
    /// GxEPD2's `_PowerOn` doing nothing.
    #[test]
    fn moving_the_0x22_guard_stops_gxepd2_powering_the_booster_on() {
        let drive = |dev: &mut GenericDisplay| {
            SpiDevice::set_dc_source(dev, 0x4000_0000, 0);
            dev.set_dc_level(false);
            dev.transfer(0x22);
            dev.set_dc_level(true);
            dev.transfer(0xF8);
        };
        let mut stock = embedded("ssd1680_tricolor_290").expect("embedded");
        drive(&mut stock);
        assert!(stock.display_on(), "0x22 0xF8 powers the booster on");

        let sabotaged = epd_yaml_with(
            "ssd1680_tricolor_290",
            (
                "when: { arg: 0, equals: 0xF8 }",
                "when: { arg: 0, equals: 0xF9 }",
            ),
        );
        let mut moved = GenericDisplay::from_yaml(&sabotaged).expect("still valid");
        drive(&mut moved);
        assert!(
            !moved.display_on(),
            "a guard on the wrong value leaves _PowerOn a no-op",
        );
    }

    /// A `when` guard reading a parameter the command does not take is refused.
    #[test]
    fn a_parameter_guard_past_the_parameter_count_is_refused() {
        let sabotaged = epd_yaml_with(
            "ssd1680_tricolor_290",
            (
                "when: { arg: 0, equals: 0xF8 }",
                "when: { arg: 3, equals: 0xF8 }",
            ),
        );
        let err = GenericDisplay::from_yaml(&sabotaged).expect_err("arg 3 of a 1-arg command");
        assert!(
            format!("{err:#}").contains("`when` reads parameter 3"),
            "unexpected error: {err:#}"
        );
    }

    /// `window_counted` BOUNDS the stream. Without it the counters wrap and the
    /// bytes after a full plane overwrite the rows just written.
    #[test]
    fn a_counted_stream_shuts_at_the_window_end() {
        let mut dev = embedded("ssd1680_tricolor_290").expect("embedded");
        SpiDevice::set_dc_source(&mut dev, 0x4000_0000, 0);
        // Window one byte-column by two rows: a two-byte stream.
        for (op, params) in [
            (0x44u8, &[0x00u8, 0x00][..]),
            (0x45, &[0x00, 0x00, 0x01, 0x00][..]),
            (0x24, &[][..]),
        ] {
            dev.set_dc_level(false);
            dev.transfer(op);
            dev.set_dc_level(true);
            for p in params {
                dev.transfer(*p);
            }
        }
        dev.set_dc_level(true);
        for b in [0x00u8, 0x00, 0xA5, 0xA5] {
            dev.transfer(b);
        }
        let black = dev.planes().ram("black").expect("black plane").to_vec();
        assert_eq!(black[0], 0x00, "row 0 written");
        assert_eq!(black[16], 0x00, "row 1 written");
        assert_eq!(
            (black[0], black[16]),
            (0x00, 0x00),
            "the two bytes past the window were DROPPED, not wrapped over the window",
        );
        assert_eq!(
            dev.planes().ink_bytes("black"),
            Some(2),
            "exactly the window"
        );
    }

    fn ssd1306_yaml_with(replacement: (&str, &str)) -> String {
        let yaml = labwired_config::embedded_device_yaml("oled-ssd1306").expect("embedded");
        let out = yaml.replace(replacement.0, replacement.1);
        assert_ne!(out, yaml, "the edit '{}' did not apply", replacement.0);
        out
    }

    #[test]
    fn a_do_entry_that_sets_no_action_is_refused() {
        // A mistyped action name parses as an all-`None` entry, which would run
        // silently and paint nothing.
        let broken = ssd1306_yaml_with((
            "do: [ { display_on: false } ]",
            "do: [ { dispay_on: false } ]",
        ));
        let err = GenericDisplay::from_yaml(&broken).expect_err("empty action must be refused");
        assert!(
            format!("{err:#}").contains("sets no action"),
            "got: {err:#}"
        );
    }

    #[test]
    fn a_do_entry_that_sets_two_actions_is_refused() {
        let broken = ssd1306_yaml_with((
            "do: [ { display_on: false } ]",
            "do: [ { display_on: false, invert: true } ]",
        ));
        let err = GenericDisplay::from_yaml(&broken).expect_err("two actions must be refused");
        assert!(format!("{err:#}").contains("2 actions"), "got: {err:#}");
    }

    #[test]
    fn an_args_index_past_the_parameter_count_is_refused() {
        let broken = ssd1306_yaml_with((
            "do: [ { set_mode: { args: [0], mask: 0x03 } } ]",
            "do: [ { set_mode: { args: [1], mask: 0x03 } } ]",
        ));
        let err = GenericDisplay::from_yaml(&broken).expect_err("bad args index must be refused");
        assert!(format!("{err:#}").contains("args index 1"), "got: {err:#}");
    }

    #[test]
    fn an_opcode_claimed_twice_is_refused() {
        let broken = ssd1306_yaml_with((
            "- { opcode: 0xAE, name: DISPLAYOFF, do: [ { display_on: false } ] }",
            "- { opcode: 0xAE, name: DISPLAYOFF, do: [ { display_on: false } ] }\n      - { opcode: 0xAE, name: AGAIN, do: [ { display_on: true } ] }",
        ));
        let err = GenericDisplay::from_yaml(&broken).expect_err("duplicate opcode must be refused");
        assert!(format!("{err:#}").contains("claimed twice"), "got: {err:#}");
    }

    #[test]
    fn a_set_var_naming_an_undeclared_cell_is_refused() {
        let yaml = labwired_config::embedded_device_yaml("st7789-170x320").expect("embedded");
        let broken = yaml.replace("name: madctl, value:", "name: mdactl, value:");
        assert_ne!(broken, yaml, "the sabotage did not apply");
        let err = GenericDisplay::from_yaml(&broken).expect_err("unknown var must be refused");
        assert!(
            format!("{err:#}").contains("not a declared var"),
            "got: {err:#}"
        );
    }

    /// An axis a layout does not have must be refused rather than quietly
    /// tracked: a `row` cursor on a page-major panel would move a counter that
    /// nothing reads, so the command would look implemented and do nothing.
    #[test]
    fn an_axis_the_layout_does_not_have_is_refused() {
        let broken = ssd1306_yaml_with((
            "- set_cursor: { axis: page, value: { opcode_mask: 0x07 } }",
            "- set_cursor: { axis: row, value: { opcode_mask: 0x07 } }",
        ));
        let err = GenericDisplay::from_yaml(&broken).expect_err("bad axis must be refused");
        assert!(
            format!("{err:#}").contains("has no meaning"),
            "got: {err:#}"
        );
    }

    /// A panel that frames on a pad is selected by CS and has no I²C address;
    /// one that frames on a control byte needs one. Getting this wrong would
    /// attach a display to a bus it cannot answer on.
    #[test]
    fn framing_and_bus_must_agree() {
        let broken = ssd1306_yaml_with(("source: control_byte", "source: pin"));
        let err = GenericDisplay::from_yaml(&broken).expect_err("mismatched framing");
        assert!(
            format!("{err:#}").contains("selected by CS"),
            "got: {err:#}"
        );
    }

    /// Horizontal addressing wraps the column into the next page and the page
    /// back to the window start. Driven here directly so the wrap is proved on
    /// the ENGINE, not only through a panel's descriptor.
    #[test]
    fn horizontal_addressing_wraps_column_into_page_and_page_into_start() {
        let mut d = ssd1306(0x3C);
        d.start();
        d.write(0x00);
        for b in [0x21u8, 0x00, 0x01, 0x22, 0x00, 0x01] {
            d.write(b);
        }
        d.start();
        d.write(0x40);
        // A 2x2 window holds four bytes; the fifth must wrap back to the start.
        for b in [0x11u8, 0x22, 0x33, 0x44, 0x55] {
            d.write(b);
        }
        d.stop();
        let fb = d.framebuffer();
        assert_eq!(
            [fb[0], fb[1], fb[128], fb[129]],
            [0x55, 0x22, 0x33, 0x44],
            "the fifth byte must overwrite the first, not run off the window"
        );
    }
}
