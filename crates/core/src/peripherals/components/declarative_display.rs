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
    DisplayCsSelect, DisplayCursorPart, DisplayDcSource, DisplayMetaFlag, DisplayPageWrap,
    DisplayPixelFormat, DisplayRamLayout, DisplayRamStream, DisplaySpec, DisplayValue,
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

    // ── identity ────────────────────────────────────────────────────────
    address: u8,
    cs_pin: String,
    dc_pin: Option<String>,
    dc_source: Option<(u64, u8)>,
    dc_level: bool,
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

    // ── protocol state ──────────────────────────────────────────────────
    framing: Framing,
    pending_cmd: u8,
    /// The command-table entry the pending opcode resolved to. Held rather
    /// than re-looked-up when the parameters complete, because a command may
    /// change the very var its own guard reads.
    pending_idx: Option<u16>,
    params: [u8; 8],
    param_have: u8,
    param_want: u8,
    unit: [u8; 4],
    unit_have: usize,
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
            w * pages
        }
        (DisplayPixelFormat::MonoPage, DisplayRamLayout::RowMajor) => {
            bail!("mono_page pixels are page-major by construction; ram.layout says row_major")
        }
        (fmt, DisplayRamLayout::RowMajor) => w * h * fmt.write_unit_bytes(),
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
        DisplayDcSource::Pin => {
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
            if spec.dc.source == DisplayDcSource::Pin && spec.commands.iter().any(|c| c.args > 0) {
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
        DisplayRamStream::Command => {
            if !has_ram_write {
                bail!(
                    "ram.stream `command` means a `ram_write` action opens the pixel stream, and \
                     the command table declares none — no data byte could ever reach frame memory"
                );
            }
        }
    }

    if spec.artifact_meta.is_empty() {
        bail!(
            "artifact_meta is empty: the paint artifact would carry pixels and no panel state, \
             so a dark frame could not explain itself"
        );
    }
    let mut meta_keys: Vec<&str> = Vec::new();
    for field in &spec.artifact_meta {
        let key = field.key();
        if matches!(key, "w" | "h" | "format" | "generation") {
            bail!("artifact_meta publishes '{key}', which describes the payload and is always present");
        }
        if meta_keys.contains(&key) {
            bail!("artifact_meta publishes '{key}' twice");
        }
        meta_keys.push(key);
        match field.flag() {
            DisplayMetaFlag::InkBytes | DisplayMetaFlag::LitPixels
                if spec.pixel_format != DisplayPixelFormat::MonoPage =>
            {
                bail!(
                    "artifact_meta '{key}' counts 1 bpp ink, but this panel is {:?}",
                    spec.pixel_format
                );
            }
            DisplayMetaFlag::TopColour | DisplayMetaFlag::TopColourPixels
                if spec.pixel_format != DisplayPixelFormat::Rgb565 =>
            {
                bail!(
                    "artifact_meta '{key}' reads a 16-bit pixel, but this panel is {:?}",
                    spec.pixel_format
                );
            }
            _ => {}
        }
    }

    let mut claims: Vec<Vec<usize>> = vec![Vec::new(); 256];
    for (i, cmd) in spec.commands.iter().enumerate() {
        if cmd.args as usize > 8 {
            bail!(
                "command 0x{:02X} takes {} parameters; this engine buffers 8",
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
        let ram = vec![0u8; spec.ram.bytes as usize];
        let mut dev = Self {
            by_opcode,
            width,
            height,
            pages,
            unit_bytes,
            address,
            cs_pin: String::new(),
            dc_pin: None,
            dc_source: None,
            dc_level: false,
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
            framing: Framing::Idle,
            pending_cmd: 0,
            pending_idx: None,
            params: [0; 8],
            param_have: 0,
            param_want: 0,
            unit: [0; 4],
            unit_have: 0,
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
            .unwrap_or_else(|| self.spec.width.saturating_sub(1));
        self.row_start = 0;
        self.row_end = self
            .spec
            .window
            .row_end
            .unwrap_or_else(|| self.spec.height.saturating_sub(1));
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

    /// What a camera would see: DISPON **and** awake. A panel that got DISPON
    /// but never SLPOUT is dark on the bench however full frame memory is.
    pub fn lit(&self) -> bool {
        self.powered && self.display_on && self.awake
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

    /// Map a logical (column, row) onto physical frame memory, which does not
    /// rotate.
    fn to_physical(&self, col: u16, row: u16) -> (usize, usize) {
        let (swap, mx, my) = self.orientation_bits();
        let (mut x, mut y) = if swap { (row, col) } else { (col, row) };
        if mx {
            x = self.spec.width.saturating_sub(1).saturating_sub(x);
        }
        if my {
            y = self.spec.height.saturating_sub(1).saturating_sub(y);
        }
        (x as usize, y as usize)
    }

    fn axis_max(&self, axis: DisplayAxis) -> u16 {
        let (aw, ah) = self.addressable();
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
                self.unit_have = 0;
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
                self.ram.fill(0);
            }
        }
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
        self.params = [0; 8];
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
        self.spec.dc.source == DisplayDcSource::Pin
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
    }

    fn commit_unit(&mut self) {
        match self.spec.ram.layout {
            DisplayRamLayout::PageMajor => {
                let idx = self.page as usize * self.width + self.col as usize;
                if idx < self.ram.len() {
                    self.ram[idx] = self.unit[0];
                }
            }
            DisplayRamLayout::RowMajor => {
                let (x, y) = self.to_physical(self.col, self.row);
                if x < self.width && y < self.height {
                    let idx = (y * self.width + x) * self.unit_bytes;
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
                if (self.col as usize) < self.width.saturating_sub(1) {
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
            let value = match field.flag() {
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

    /// The frame memory, for a snapshot. Only the pixels: control state is
    /// rebuilt by replaying the bus, and a snapshot that carried a cursor
    /// would resume a half-written frame at a position the wire never sent.
    fn snapshot_ram(&self) -> Vec<u8> {
        self.ram.clone()
    }

    fn restore_ram(&mut self, bytes: &[u8]) {
        if bytes.len() == self.ram.len() {
            self.ram.copy_from_slice(bytes);
        }
    }
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
        self.restore_ram(bytes);
        Ok(())
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
        DisplayDcSource::Pin => (Transport::Spi, Category::Spi),
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
            DisplayDcSource::Pin => {
                dev.set_cs_pin(ctx.config_str("cs_pin").unwrap_or("").to_string());
                let dc = ctx
                    .config_str("dc_pin")
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "{} '{}': no `dc_pin`. This panel frames commands from the D/C line \
                             and has no infer-from-byte-values fallback: that inference decodes a \
                             parameter byte 0x2C as RAMWR and writes the remaining init bytes \
                             into the framebuffer as pixels, leaving a blank screen and a \
                             blameless firmware.",
                            self.descriptor.r#type,
                            ctx.device_id(),
                        )
                    })?;
                // Resolving the pin to its GPIO output register is the half that
                // makes D/C real: the bus samples that register before each
                // transfer. Declaring the pin without this leaves D/C stuck low,
                // every byte frames as a command, and the panel renders blank
                // with no error.
                let (odr_addr, bit) = ctx.resolve_pin_odr(&dc).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{} '{}': D/C pin '{}' does not resolve to a driveable GPIO output.",
                        self.descriptor.r#type,
                        ctx.device_id(),
                        dc,
                    )
                })?;
                dev.set_dc_pin(dc);
                SpiDevice::set_dc_source(&mut dev, odr_addr, bit);

                if spec.supply_gated && ctx.config_bool("powered") == Some(false) {
                    dev.set_powered(false);
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
