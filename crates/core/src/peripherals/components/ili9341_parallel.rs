// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ILI9341 8080-style parallel (bit-bang) panel twin.
//!
//! Phase-2 v1 targets ESP32 / ESP32-S3 GPIO bit-bang of the classic 16-bit
//! Intel 8080 bus (CS, RS/D-C, WR, RD, RST, DB[15:0]). Edges arrive through
//! [`GpioObserver`](crate::peripherals::device::GpioObserver); unit tests
//! inject them directly.
//!
//! ## Bus protocol (write path)
//!
//! - **CS active low.** While CS is high, WR edges are ignored.
//! - **WR falling edge** (high→low) while CS is low samples RS and DB[15:0]:
//!   - RS low → command (low 8 bits of the bus)
//!   - RS high → data / pixel stream
//! - **RST low** clears the framebuffer and resets the addressing window
//!   (hardware reset; distinct from SPI SWRESET which leaves frame memory).
//!
//! Commands supported for paint: CASET (`0x2A`), PASET (`0x2B`), RAMWR
//! (`0x2C`), DISPON (`0x29`), SWRESET (`0x01`), MADCTL (`0x36`), COLMOD
//! (`0x3A`). Framebuffer is 240×320 RGB565 big-endian, matching the SPI kit.
//!
//! Interior mutability: the observer hook is `&self`, so protocol + FB state
//! live behind a `Mutex`. Hold as `Arc<Ili9341Parallel>` when attaching to GPIO.

use std::sync::Mutex;

const WIDTH: usize = 240;
const HEIGHT: usize = 320;
const FB_BYTES: usize = WIDTH * HEIGHT * 2; // RGB565, 2 bytes per pixel

const MADCTL_MY: u8 = 0x80;
const MADCTL_MX: u8 = 0x40;
const MADCTL_MV: u8 = 0x20;

/// GPIO numbers for the 8080 control + data bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelPins {
    pub cs: u8,
    pub rs: u8,
    pub wr: u8,
    pub rd: u8,
    pub rst: u8,
    /// Data bus pins, DB0..DB15 (DB0 is LSB).
    pub db: [u8; 16],
}

/// Latched pad levels + ILI9341 command/pixel state.
#[derive(Debug)]
struct State {
    cs: bool,
    rs: bool,
    wr: bool,
    rd: bool,
    rst: bool,
    /// Latched DB[15:0] (bit i = pin `db[i]`).
    db: u16,

    display_on: bool,
    cur_col: u16,
    cur_row: u16,
    col_start: u16,
    col_end: u16,
    row_start: u16,
    row_end: u16,
    framebuffer: Vec<u8>,
    madctl: u8,
    cur_cmd: u8,
    param_buf: [u8; 4],
    param_len: usize,
    in_ramwr: bool,
}

impl State {
    fn new() -> Self {
        Self {
            // Idle: CS/WR/RD/RST high, RS low, data zero.
            cs: true,
            rs: false,
            wr: true,
            rd: true,
            rst: true,
            db: 0,
            display_on: false,
            cur_col: 0,
            cur_row: 0,
            col_start: 0,
            col_end: (WIDTH as u16) - 1,
            row_start: 0,
            row_end: (HEIGHT as u16) - 1,
            framebuffer: vec![0u8; FB_BYTES],
            madctl: 0,
            cur_cmd: 0,
            param_buf: [0; 4],
            param_len: 0,
            in_ramwr: false,
        }
    }

    fn addressable_width(&self) -> u16 {
        if self.madctl & MADCTL_MV != 0 {
            HEIGHT as u16
        } else {
            WIDTH as u16
        }
    }

    fn addressable_height(&self) -> u16 {
        if self.madctl & MADCTL_MV != 0 {
            WIDTH as u16
        } else {
            HEIGHT as u16
        }
    }

    fn to_physical(&self, col: u16, row: u16) -> (usize, usize) {
        let (mut x, mut y) = if self.madctl & MADCTL_MV != 0 {
            (row, col)
        } else {
            (col, row)
        };
        if self.madctl & MADCTL_MX != 0 {
            x = (WIDTH as u16).saturating_sub(1).saturating_sub(x);
        }
        if self.madctl & MADCTL_MY != 0 {
            y = (HEIGHT as u16).saturating_sub(1).saturating_sub(y);
        }
        (x as usize, y as usize)
    }

    fn hard_reset(&mut self) {
        self.display_on = false;
        self.cur_col = 0;
        self.cur_row = 0;
        self.col_start = 0;
        self.col_end = (WIDTH as u16) - 1;
        self.row_start = 0;
        self.row_end = (HEIGHT as u16) - 1;
        self.framebuffer.fill(0);
        self.madctl = 0;
        self.cur_cmd = 0;
        self.param_buf = [0; 4];
        self.param_len = 0;
        self.in_ramwr = false;
    }

    fn param_count(cmd: u8) -> usize {
        match cmd {
            0x2A | 0x2B => 4, // CASET / PASET
            0x36 | 0x3A => 1, // MADCTL / COLMOD
            _ => 0,
        }
    }

    fn apply_simple_command(&mut self, cmd: u8) {
        match cmd {
            0x01 => {
                // SWRESET — reset window / display state. Frame memory left
                // alone (datasheet); hardware RST clears memory separately.
                self.col_start = 0;
                self.col_end = self.addressable_width() - 1;
                self.row_start = 0;
                self.row_end = self.addressable_height() - 1;
                self.display_on = false;
                self.madctl = 0;
            }
            0x28 => self.display_on = false,
            0x29 => self.display_on = true,
            _ => {}
        }
    }

    fn on_command(&mut self, cmd: u8) {
        self.cur_cmd = cmd;
        self.param_len = 0;
        self.param_buf = [0; 4];
        match cmd {
            0x2C => {
                // RAMWR — open pixel stream at window origin.
                self.cur_col = self.col_start;
                self.cur_row = self.row_start;
                self.in_ramwr = true;
            }
            0x3C => {
                // RAMWR continue — resume without resetting pointer.
                self.in_ramwr = true;
            }
            _ => {
                self.in_ramwr = false;
                self.apply_simple_command(cmd);
            }
        }
    }

    fn on_data_byte(&mut self, byte: u8) {
        if self.param_len < self.param_buf.len() {
            self.param_buf[self.param_len] = byte;
        }
        self.param_len += 1;
        let want = Self::param_count(self.cur_cmd);
        if want > 0 && self.param_len == want {
            let (cmd, params) = (self.cur_cmd, self.param_buf);
            self.handle_params_complete(cmd, &params);
        }
    }

    fn handle_params_complete(&mut self, cmd: u8, params: &[u8; 4]) {
        match cmd {
            0x2A => {
                let start = ((params[0] as u16) << 8) | (params[1] as u16);
                let end = ((params[2] as u16) << 8) | (params[3] as u16);
                let limit = self.addressable_width() - 1;
                self.col_start = start.min(limit);
                self.col_end = end.min(limit);
            }
            0x2B => {
                let start = ((params[0] as u16) << 8) | (params[1] as u16);
                let end = ((params[2] as u16) << 8) | (params[3] as u16);
                let limit = self.addressable_height() - 1;
                self.row_start = start.min(limit);
                self.row_end = end.min(limit);
            }
            0x36 => {
                self.madctl = params[0];
            }
            _ => {}
        }
    }

    fn write_pixel_u16(&mut self, pixel: u16) {
        let hi = (pixel >> 8) as u8;
        let lo = (pixel & 0xFF) as u8;
        let (x, y) = self.to_physical(self.cur_col, self.cur_row);
        let idx = (y * WIDTH + x) * 2;
        if x < WIDTH && idx + 1 < self.framebuffer.len() {
            self.framebuffer[idx] = hi;
            self.framebuffer[idx + 1] = lo;
        }
        if self.cur_col >= self.col_end {
            self.cur_col = self.col_start;
            if self.cur_row >= self.row_end {
                self.cur_row = self.row_start;
            } else {
                self.cur_row += 1;
            }
        } else {
            self.cur_col += 1;
        }
    }

    /// WR falling edge while CS is low: sample RS + DB.
    fn on_wr_strobe(&mut self) {
        let bus = self.db;
        if !self.rs {
            // Command — low 8 bits (ILI9341 command register is 8-bit).
            self.on_command((bus & 0xFF) as u8);
        } else if self.in_ramwr {
            // One 16-bit RGB565 pixel per WR on a 16-bit 8080 bus.
            self.write_pixel_u16(bus);
        } else {
            // Parameter stream as successive 8-bit values on D[7:0].
            self.on_data_byte((bus & 0xFF) as u8);
        }
    }
}

/// Simulated ILI9341 driven by GPIO bit-bang of an 8080 parallel bus.
#[derive(Debug)]
pub struct Ili9341Parallel {
    pins: ParallelPins,
    state: Mutex<State>,
    id: String,
}

impl Ili9341Parallel {
    pub fn new(id: impl Into<String>, pins: ParallelPins) -> Self {
        Self {
            pins,
            state: Mutex::new(State::new()),
            id: id.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn pins(&self) -> &ParallelPins {
        &self.pins
    }

    /// Non-zero framebuffer bytes (paint evidence for tests / inspect).
    pub fn ink_bytes(&self) -> usize {
        let s = self.state.lock().unwrap();
        s.framebuffer.iter().filter(|&&b| b != 0).count()
    }

    /// Snapshot of the raw RGB565 framebuffer (row-major, big-endian per pixel).
    pub fn framebuffer(&self) -> Vec<u8> {
        self.state.lock().unwrap().framebuffer.clone()
    }

    pub fn display_on(&self) -> bool {
        self.state.lock().unwrap().display_on
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }

    /// Addressable (logical) size under the current MADCTL. `MADCTL_MV` swaps
    /// the axes, so a landscape-configured panel is 320×240 even though the
    /// physical frame memory stays 240×320.
    pub fn logical_dimensions(&self) -> (usize, usize) {
        let s = self.state.lock().unwrap();
        (
            s.addressable_width() as usize,
            s.addressable_height() as usize,
        )
    }

    /// Framebuffer with MADCTL applied for host row-major rendering (same idea
    /// as the SPI [`super::ili9341::Ili9341::oriented_framebuffer`]).
    ///
    /// The result is [`Self::logical_dimensions`] wide × high — i.e. it is
    /// indexed by the CASET/PASET coordinates the firmware wrote, which is the
    /// orientation a viewer expects. Row stride is the LOGICAL width: iterating
    /// the physical 240×320 extents here (as this used to) walks off the end of
    /// each row under `MADCTL_MV` and shears the image.
    pub fn oriented_framebuffer(&self) -> Vec<u8> {
        let s = self.state.lock().unwrap();
        let lw = s.addressable_width() as usize;
        let lh = s.addressable_height() as usize;
        let mut out = vec![0u8; lw * lh * 2];
        for row in 0..lh {
            for col in 0..lw {
                let (x, y) = s.to_physical(col as u16, row as u16);
                let src = (y * WIDTH + x) * 2;
                let dst = (row * lw + col) * 2;
                if src + 1 < s.framebuffer.len() {
                    out[dst] = s.framebuffer[src];
                    out[dst + 1] = s.framebuffer[src + 1];
                }
            }
        }
        out
    }

    /// Drive one 8080 bus cycle from a *peripheral* instead of from GPIO edges.
    ///
    /// The ESP32-S3 `LCD_CAM` i80 master owns DB[15:0], WR and D/C once the
    /// firmware routes them through the GPIO matrix (`esp_lcd_new_i80_bus`), so
    /// the pads never toggle as CPU-visible GPIO and [`Self::on_gpio_edge`]
    /// never fires. This is the same latch [`State::on_wr_strobe`] performs —
    /// sample D/C and DB[15:0] on the WR falling edge — with the bus word and
    /// the D/C level supplied by the controller.
    ///
    /// CS is deliberately not consulted: on the i80 path CS is a peripheral
    /// output asserted by the LCD_CAM state machine for the duration of the
    /// transaction, not a pad the firmware drives, so there is no CS edge to
    /// latch. A transaction reaching here *is* the chip-select assertion.
    ///
    /// `dc_high` is the D/C (RS) level for this cycle: `false` = command phase,
    /// `true` = data phase.
    pub fn i80_write_word(&self, dc_high: bool, word: u16) {
        let mut s = self.state.lock().unwrap();
        // Latch the pad state a real strobe would leave behind, so a firmware
        // that mixes the two paths sees a consistent bus.
        s.rs = dc_high;
        s.db = word;
        s.on_wr_strobe();
    }

    /// Feed one GPIO transition. Unit tests and the ESP32/S3 observers call this.
    pub fn on_gpio_edge(&self, pin: u8, to: bool, _sim_cycle: u64) {
        let mut s = self.state.lock().unwrap();
        let p = &self.pins;

        if pin == p.cs {
            s.cs = to;
            return;
        }
        if pin == p.rs {
            s.rs = to;
            return;
        }
        if pin == p.rd {
            s.rd = to;
            return;
        }
        if pin == p.rst {
            let was = s.rst;
            s.rst = to;
            if was && !to {
                // Falling edge on RST → hardware reset (clears FB).
                s.hard_reset();
            }
            return;
        }
        if pin == p.wr {
            let was = s.wr;
            s.wr = to;
            // Falling edge while selected (CS low).
            if was && !to && !s.cs {
                s.on_wr_strobe();
            }
            return;
        }

        // Data bus pin?
        if let Some(bit) = p.db.iter().position(|&d| d == pin) {
            if to {
                s.db |= 1u16 << bit;
            } else {
                s.db &= !(1u16 << bit);
            }
        }
    }
}

impl crate::peripherals::device::GpioObserver for Ili9341Parallel {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

// ─── PeripheralKit (universal attach) ──────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

/// Kit for the 16-bit parallel ILI9341 (`ili9341-16bit`). Distinct device_type
/// from the SPI kit (`ili9341`) so both can coexist in the registry.
pub struct Ili9341ParallelKit;
pub static ILI9341_PARALLEL_KIT: Ili9341ParallelKit = Ili9341ParallelKit;

static ILI9341_PARALLEL_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "ili9341-16bit",
    label: "ILI9341 TFT (16-bit parallel)",
    summary: "240×320 RGB565 TFT over Intel 8080 16-bit GPIO bit-bang.",
    detail: "LCDWiki MRB3205-class 3.2\" module contract: CS/RS/WR/RD/RST + DB0..DB15. \
             Firmware bit-bangs the bus; the twin watches GPIO edges (classic ESP32 / \
             ESP32-S3) and paints an in-memory RGB565 framebuffer. Not SPI — use \
             device_type ili9341 for the 4-wire SPI kit.",
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: &[
        ConfigKey {
            name: "cs_pin",
            ty: ConfigType::Str,
            doc: "LCD chip-select GPIO (e.g. \"GPIO15\"). Defaults to GPIO15.",
        },
        ConfigKey {
            name: "rs_pin",
            ty: ConfigType::Str,
            doc: "Register/data select GPIO (alias dc_pin). Defaults to GPIO2.",
        },
        ConfigKey {
            name: "wr_pin",
            ty: ConfigType::Str,
            doc: "Write strobe GPIO. Defaults to GPIO4.",
        },
        ConfigKey {
            name: "rd_pin",
            ty: ConfigType::Str,
            doc: "Read strobe GPIO (latched; read-back not modelled). Defaults to GPIO5.",
        },
        ConfigKey {
            name: "rst_pin",
            ty: ConfigType::Str,
            doc: "Reset GPIO (active low). Defaults to GPIO33.",
        },
        ConfigKey {
            name: "db0_pin",
            ty: ConfigType::Str,
            doc: "Data bus bit 0 (LSB). Also db1_pin..db15_pin. Defaults GPIO10..GPIO25.",
        },
    ],
    // Example system lives at examples/ili9341-16bit-lab; keep labs empty until
    // a non-empty demo_elf ships (UI kitsWithLabs requires demo_elf length > 0).
    labs: &[],
};

impl PeripheralKit for Ili9341ParallelKit {
    fn metadata(&self) -> &'static KitMetadata {
        &ILI9341_PARALLEL_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_gpio_pin("cs_pin", "CS", "GPIO15")?;
        let rs = ctx
            .config_gpio_pin("rs_pin", "RS", "GPIO2")
            .or_else(|_| ctx.config_gpio_pin("dc_pin", "DC", "GPIO2"))?;
        let wr = ctx.config_gpio_pin("wr_pin", "WR", "GPIO4")?;
        let rd = ctx.config_gpio_pin("rd_pin", "RD", "GPIO5")?;
        let rst = ctx.config_gpio_pin("rst_pin", "RST", "GPIO33")?;
        let mut db = [0u8; 16];
        for (i, pin) in db.iter_mut().enumerate() {
            let key = format!("db{i}_pin");
            let alt = format!("DB{i}");
            let default = format!("GPIO{}", 10 + i);
            *pin = ctx.config_gpio_pin(&key, &alt, &default)?;
        }
        let pins = ParallelPins {
            cs,
            rs,
            wr,
            rd,
            rst,
            db,
        };
        let panel = std::sync::Arc::new(Ili9341Parallel::new(ctx.device_id(), pins));
        // Universal GPIO bit-bang attach: same choke point as motors/servos.
        ctx.install_gpio_observer(panel.clone());
        // ESP32-S3 i80 attach: when the chip has an LCD_CAM block, bind the
        // same panel to it so firmware driving the bus through `esp_lcd`'s i80
        // master (LCD_CMD_VAL + GDMA outlink, no GPIO edges at all) paints too.
        // Both paths feed one panel model — whichever the firmware uses.
        if let Some(idx) = ctx.bus.find_peripheral_index_by_name("lcd_cam") {
            if let Some(lcd) = ctx.bus.peripherals[idx].dev.as_any_mut().and_then(|a| {
                a.downcast_mut::<crate::peripherals::esp32s3::lcd_cam::Esp32s3LcdCam>()
            }) {
                lcd.attach_panel(panel.clone());
            }
        }
        ctx.bus.observe_device(panel);
        Ok(())
    }
}

/// Bus-resident parallel panel reports the same RGB565 framebuffer evidence as
/// the SPI kit so inspect / `display_artifact` work without a SPI controller.
impl crate::inspect::DeviceEvidence for Ili9341Parallel {
    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        let fb = self.oriented_framebuffer();
        let painted = fb.iter().filter(|&&b| b != 0x00).count();
        // Logical extents: `oriented_framebuffer` is in CASET/PASET space, so
        // a landscape MADCTL reports 320×240 and the bytes match the stride.
        let (w, h) = self.logical_dimensions();
        let mut counts: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for px in fb.chunks_exact(2) {
            let v = u16::from_be_bytes([px[0], px[1]]);
            if v != 0 {
                *counts.entry(v).or_default() += 1;
            }
        }
        let top = counts.iter().max_by_key(|&(_, n)| *n);
        vec![crate::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": w,
                "h": h,
                "format": crate::inspect::artifact_format::RGB565_BE,
                "generation": crate::inspect::artifact_generation(&fb),
                "display_on": self.display_on(),
                "painted_bytes": painted,
                "total_bytes": fb.len(),
                "top_colour": top.map(|(v, _)| format!("0x{v:04X}")),
                "top_colour_pixels": top.map(|(_, n)| *n),
                "bus": "8080-16bit",
            }),
            bytes: crate::inspect::artifact_bytes(&fb, opts),
        }]
    }
}

/// A bus-resident DISPLAY: readback only as far as the bus is concerned, but
/// it reports its RGB565 framebuffer as evidence, the same shape the SPI kit's
/// panels emit.
impl crate::bus::ObservedDevice for Ili9341Parallel {
    fn manifest_id(&self) -> &str {
        self.id()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Default pin map used by unit tests (GPIO numbers are arbitrary).
    fn test_pins() -> ParallelPins {
        ParallelPins {
            cs: 0,
            rs: 1,
            wr: 2,
            rd: 3,
            rst: 4,
            // DB0..DB15 → GPIO 10..25
            db: [
                10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
            ],
        }
    }

    fn panel() -> Ili9341Parallel {
        Ili9341Parallel::new("ili9341-par", test_pins())
    }

    fn set_bus(p: &Ili9341Parallel, value: u16) {
        let pins = p.pins();
        for bit in 0..16u8 {
            let level = (value >> bit) & 1 != 0;
            p.on_gpio_edge(pins.db[bit as usize], level, 0);
        }
    }

    /// Pulse WR low→high while holding the bus (falling edge samples).
    fn strobe_wr(p: &Ili9341Parallel) {
        let wr = p.pins().wr;
        // Ensure WR is high before the falling edge (idle is already high).
        p.on_gpio_edge(wr, true, 0);
        p.on_gpio_edge(wr, false, 0);
        p.on_gpio_edge(wr, true, 0);
    }

    fn write_cmd(p: &Ili9341Parallel, cmd: u8) {
        let pins = p.pins();
        p.on_gpio_edge(pins.rs, false, 0); // command
        set_bus(p, cmd as u16);
        strobe_wr(p);
    }

    fn write_data8(p: &Ili9341Parallel, byte: u8) {
        let pins = p.pins();
        p.on_gpio_edge(pins.rs, true, 0); // data
        set_bus(p, byte as u16);
        strobe_wr(p);
    }

    fn write_data16(p: &Ili9341Parallel, word: u16) {
        let pins = p.pins();
        p.on_gpio_edge(pins.rs, true, 0);
        set_bus(p, word);
        strobe_wr(p);
    }

    fn select(p: &Ili9341Parallel) {
        p.on_gpio_edge(p.pins().cs, false, 0);
    }

    fn deselect(p: &Ili9341Parallel) {
        p.on_gpio_edge(p.pins().cs, true, 0);
    }

    #[test]
    fn ramwr_pixels_produce_ink() {
        let p = panel();
        select(&p);
        write_cmd(&p, 0x29); // DISPON
        write_cmd(&p, 0x2C); // RAMWR
        write_data16(&p, 0xF800); // one red RGB565 pixel
        assert!(
            p.ink_bytes() > 0,
            "expected non-zero ink after RAMWR + pixel, got {}",
            p.ink_bytes()
        );
        assert!(p.display_on(), "DISPON should latch display_on");
        let fb = p.framebuffer();
        assert_eq!(fb.len(), FB_BYTES);
        assert_eq!(fb[0], 0xF8);
        assert_eq!(fb[1], 0x00);
    }

    #[test]
    fn cs_high_ignores_wr() {
        let p = panel();
        // CS left high (deselected) — WR strobes must not paint.
        deselect(&p);
        write_cmd(&p, 0x2C);
        write_data16(&p, 0xF800);
        assert_eq!(
            p.ink_bytes(),
            0,
            "CS high must ignore WR; ink={}",
            p.ink_bytes()
        );
    }

    #[test]
    fn rst_clears_framebuffer() {
        let p = panel();
        select(&p);
        write_cmd(&p, 0x2C);
        write_data16(&p, 0x07E0); // green
        assert!(p.ink_bytes() > 0);

        // Hardware reset: RST falling edge clears FB.
        let rst = p.pins().rst;
        p.on_gpio_edge(rst, true, 0);
        p.on_gpio_edge(rst, false, 0);
        assert_eq!(p.ink_bytes(), 0, "RST must clear framebuffer");
        assert!(!p.display_on());
        // Window reset to full portrait.
        select(&p);
        write_cmd(&p, 0x2C);
        write_data16(&p, 0x001F); // blue at origin
        let fb = p.framebuffer();
        assert_eq!(fb[0], 0x00);
        assert_eq!(fb[1], 0x1F);
    }

    #[test]
    fn caset_paset_window_places_pixel() {
        let p = panel();
        select(&p);
        // CASET: columns 2..=2
        write_cmd(&p, 0x2A);
        write_data8(&p, 0x00);
        write_data8(&p, 0x02);
        write_data8(&p, 0x00);
        write_data8(&p, 0x02);
        // PASET: rows 3..=3
        write_cmd(&p, 0x2B);
        write_data8(&p, 0x00);
        write_data8(&p, 0x03);
        write_data8(&p, 0x00);
        write_data8(&p, 0x03);
        write_cmd(&p, 0x2C);
        write_data16(&p, 0xABCD);

        let fb = p.framebuffer();
        let idx = (3 * WIDTH + 2) * 2;
        assert_eq!(fb[idx], 0xAB);
        assert_eq!(fb[idx + 1], 0xCD);
        // Origin untouched.
        assert_eq!(fb[0], 0);
        assert_eq!(fb[1], 0);
    }
}
