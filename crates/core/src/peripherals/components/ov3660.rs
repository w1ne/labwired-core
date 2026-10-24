// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! OmniVision **OV3660** 3 MP CMOS camera — SCCB control port + DVP pixel
//! output.
//!
//! The part is two interfaces with one register file behind them:
//!
//! * **SCCB** (I²C-compatible, 7-bit address `0x3C`, 16-bit register
//!   addresses, 8-bit data). A write transaction is `[addr_hi, addr_lo,
//!   data…]` with the pointer auto-incrementing per data byte; a read is a
//!   pointer-setting write, a STOP, then a read transaction that returns
//!   `reg[ptr]` and increments. This is exactly the framing
//!   `esp32-camera`'s `SCCB_Write16` / `SCCB_Read16` put on the wire.
//! * **DVP** (8-bit parallel). [`DvpSensor::next_frame`] renders one frame
//!   from the CURRENT register state: the output window the driver programmed
//!   (`0x3808`–`0x380B`), the format it selected (`0x4300`), the colour-bar
//!   test-pattern enable (`0x503D[7]`), the JPEG compression enable
//!   (`0x3821[5]`) and quality (`0x4407`), and the power-down bit.
//!
//! ## What the frame shows
//!
//! There is no lens and no world, so the scene is synthetic and driven by
//! stimulus — `pattern` (colour bars / checkerboard / gradient / flat grey),
//! `brightness` (0–100 %) and `motion` (pixels the scene scrolls per frame, so
//! successive frames differ and a capture loop is visibly live). The pre-ISP
//! test-pattern bit wins over the stimulus, as it does on silicon: firmware
//! that calls `set_colorbar(1)` gets bars whatever the scene is.
//!
//! ## JPEG
//!
//! With the compression enable (`0x3821[5]`) set, each frame is a real
//! baseline JPEG of the same scene ([`crate::peripherals::jpeg_baseline`]):
//! YCbCr 4:2:2 at the programmed window, quantised by `COMPRESSION_CTRL07`
//! (`0x4407[5:0]`, the QS `esp32-camera`'s `set_quality` writes; lower is
//! better, mapped onto the 1–100 scale as `100 - 1.5 × QS`). The bytes on
//! D[7:0] are then sized for the driver that reads them, as a sensor's are:
//!
//! * `esp32-camera` sizes a PSRAM JPEG frame as the number of 1 KiB DMA
//!   blocks it saw and looks for `FF D9` only in the last one, so a JPEG
//!   whose length is an exact multiple of 1024 could never be found. That one
//!   case gets a 4-byte COM segment after SOI, which moves EOI off the
//!   boundary and is ignored by every decoder.
//! * The stream is then zero-padded to one byte short of the next 1 KiB
//!   boundary, so the last DMA block holds no stale bytes from a larger
//!   earlier frame for the driver's backward `FF D9` search to trip over.
//!
//! The JPEG is an encoding of the synthetic scene, not of the OV3660's own
//! compression engine: quantisation, chroma sampling and file size are
//! representative, not bit-exact.
//!
//! ## What is NOT modelled
//!
//! * Exposure, gain, white balance, gamma, scaling quality, PLL and the
//!   ISP — the registers round-trip (drivers read-modify-write them) but have
//!   no effect on the pixels.
//! * Register reset values other than the chip ID, the system-control
//!   register and the output window/format. Everything else resets to 0; every
//!   driver in the wild loads a full init table after the software reset, so
//!   no firmware observes the difference.
//!
//! Register addresses and bit meanings follow the OV3660 register map as
//! transcribed in Espressif's `esp32-camera` (`sensors/private_include/
//! ov3660_regs.h`), which is the map the firmware this twin targets is
//! written against.

use std::sync::{Arc, Mutex};

use super::dvp_sensor::DvpSensor;
use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::jpeg_baseline;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// 7-bit SCCB address. The OV3660 has no address-select pin.
pub const OV3660_SCCB_ADDR: u8 = 0x3C;
/// Product ID read back from `0x300A`/`0x300B`.
pub const OV3660_PID: u16 = 0x3660;

/// SYSTEM_CTROL0: bit 7 software reset (self-clearing), bit 6 software
/// power-down.
const REG_SYSTEM_CTROL0: u16 = 0x3008;
const SYS_SOFT_RESET: u8 = 0x80;
const SYS_POWER_DOWN: u8 = 0x40;
/// Chip ID, read-only.
const REG_CHIP_ID_H: u16 = 0x300A;
const REG_CHIP_ID_L: u16 = 0x300B;
/// Output window size, 12-bit width/height split high/low.
const REG_X_OUTPUT_SIZE_H: u16 = 0x3808;
const REG_X_OUTPUT_SIZE_L: u16 = 0x3809;
const REG_Y_OUTPUT_SIZE_H: u16 = 0x380A;
const REG_Y_OUTPUT_SIZE_L: u16 = 0x380B;
/// TIMING_TC_REG21: bit 5 enables JPEG compression.
const REG_TIMING_TC_REG21: u16 = 0x3821;
const TC21_JPEG_EN: u8 = 0x20;
/// COMPRESSION_CTRL07: [5:0] QS, the JPEG quantisation scale (lower = better).
const REG_COMPRESSION_CTRL07: u16 = 0x4407;
/// The DMA block `esp32-camera` sizes JPEG frames by (`dma_half_buffer_size`).
const JPEG_DMA_BLOCK: usize = 1024;
/// FORMAT_CTRL00: [7:4] output format, [3:0] byte sequence.
const REG_FORMAT_CTRL00: u16 = 0x4300;
/// PRE_ISP_TEST_SETTING_1: bit 7 enables the colour-bar test pattern.
const REG_PRE_ISP_TEST: u16 = 0x503D;
const PRE_ISP_TEST_EN: u8 = 0x80;

/// Native array, 2048 × 1536. Also the out-of-reset output window.
const MAX_W: u32 = 2048;
const MAX_H: u32 = 1536;

/// Values written at power-on and by the software reset. See the module doc
/// for why the list is short.
const RESET_VALUES: &[(u16, u8)] = &[
    (REG_SYSTEM_CTROL0, 0x02),
    (REG_CHIP_ID_H, (OV3660_PID >> 8) as u8),
    (REG_CHIP_ID_L, (OV3660_PID & 0xFF) as u8),
    (REG_X_OUTPUT_SIZE_H, (MAX_W >> 8) as u8),
    (REG_X_OUTPUT_SIZE_L, (MAX_W & 0xFF) as u8),
    (REG_Y_OUTPUT_SIZE_H, (MAX_H >> 8) as u8),
    (REG_Y_OUTPUT_SIZE_L, (MAX_H & 0xFF) as u8),
    // YUV422, YUYV.
    (REG_FORMAT_CTRL00, 0x30),
];

/// Scene selected by the `pattern` stimulus channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scene {
    ColourBars,
    Checkerboard,
    Gradient,
    Flat,
}

impl Scene {
    fn from_value(v: f64) -> Self {
        match v.round() as i64 {
            1 => Self::Checkerboard,
            2 => Self::Gradient,
            3 => Self::Flat,
            _ => Self::ColourBars,
        }
    }
}

/// The eight SMPTE-order bars: white, yellow, cyan, green, magenta, red,
/// blue, black.
const BARS: [(u8, u8, u8); 8] = [
    (255, 255, 255),
    (255, 255, 0),
    (0, 255, 255),
    (0, 255, 0),
    (255, 0, 255),
    (255, 0, 0),
    (0, 0, 255),
    (0, 0, 0),
];

/// Output encoding decoded from FORMAT_CTRL00.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutFormat {
    /// RGB565, two bytes per pixel. `rgb_order` false swaps R and B (0x60).
    Rgb565 { rgb_order: bool },
    /// YUV422, two bytes per pixel; `seq` is FORMAT_CTRL00[1:0]:
    /// 0 YUYV, 1 YVYU, 2 UYVY, 3 VYUY.
    Yuv422 { seq: u8 },
    /// Y8 greyscale, one byte per pixel.
    Y8,
}

impl OutFormat {
    fn decode(reg: u8) -> Self {
        match reg >> 4 {
            0x1 => Self::Y8,
            0x6 => Self::Rgb565 {
                rgb_order: reg & 0x0F != 0,
            },
            // 0x3 is YUV422; anything this model does not know is rendered as
            // YUV422 too rather than inventing a third encoding.
            _ => Self::Yuv422 { seq: reg & 0x03 },
        }
    }
}

#[derive(Debug)]
struct Inner {
    regs: Box<[u8]>,
    /// SCCB register pointer.
    ptr: u16,
    /// Byte index within the current SCCB write transaction.
    idx: u8,
    frame_no: u64,
    scene: Scene,
    brightness_pct: f64,
    motion_px: f64,
    /// SCCB writes and notable events, oldest first.
    log: Vec<String>,
    /// Set once the first JPEG frame has been logged, so the log stays readable.
    jpeg_logged: bool,
}

impl Inner {
    fn new() -> Self {
        let mut s = Self {
            regs: vec![0u8; 0x1_0000].into_boxed_slice(),
            ptr: 0,
            idx: 0,
            frame_no: 0,
            scene: Scene::ColourBars,
            brightness_pct: 100.0,
            motion_px: 0.0,
            log: Vec::new(),
            jpeg_logged: false,
        };
        s.reset_registers();
        s
    }

    fn reset_registers(&mut self) {
        self.regs.fill(0);
        for &(addr, v) in RESET_VALUES {
            self.regs[addr as usize] = v;
        }
    }

    fn reg(&self, addr: u16) -> u8 {
        self.regs[addr as usize]
    }

    fn write_reg(&mut self, addr: u16, v: u8) {
        self.log.push(format!("W {addr:#06x}={v:#04x}"));
        match addr {
            REG_CHIP_ID_H | REG_CHIP_ID_L => {}
            REG_SYSTEM_CTROL0 if v & SYS_SOFT_RESET != 0 => {
                self.reset_registers();
                self.regs[addr as usize] = v & !SYS_SOFT_RESET;
            }
            _ => self.regs[addr as usize] = v,
        }
    }

    fn size(&self) -> (u32, u32) {
        let w = (u32::from(self.reg(REG_X_OUTPUT_SIZE_H) & 0x0F) << 8)
            | u32::from(self.reg(REG_X_OUTPUT_SIZE_L));
        let h = (u32::from(self.reg(REG_Y_OUTPUT_SIZE_H) & 0x07) << 8)
            | u32::from(self.reg(REG_Y_OUTPUT_SIZE_L));
        (w.min(MAX_W), h.min(MAX_H))
    }

    fn pixel(&self, x: u32, y: u32, w: u32, h: u32, bars: bool) -> (u8, u8, u8) {
        let shift = (self.motion_px * self.frame_no as f64) as u64;
        let xs = ((u64::from(x) + shift) % u64::from(w.max(1))) as u32;
        let (r, g, b) = if bars {
            BARS[(xs * 8 / w.max(1)) as usize % 8]
        } else {
            match self.scene {
                Scene::ColourBars => BARS[(xs * 8 / w.max(1)) as usize % 8],
                Scene::Checkerboard => {
                    if ((xs / 16) + (y / 16)) % 2 == 0 {
                        (255, 255, 255)
                    } else {
                        (0, 0, 0)
                    }
                }
                Scene::Gradient => ((xs * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 128),
                Scene::Flat => (255, 255, 255),
            }
        };
        let k = self.brightness_pct / 100.0;
        let scale = |c: u8| (f64::from(c) * k).round().clamp(0.0, 255.0) as u8;
        (scale(r), scale(g), scale(b))
    }

    fn render(&mut self) -> Option<Vec<u8>> {
        if self.reg(REG_SYSTEM_CTROL0) & SYS_POWER_DOWN != 0 {
            return None;
        }
        let (w, h) = self.size();
        if w == 0 || h == 0 {
            return None;
        }
        let bars = self.reg(REG_PRE_ISP_TEST) & PRE_ISP_TEST_EN != 0;
        if self.reg(REG_TIMING_TC_REG21) & TC21_JPEG_EN != 0 {
            let frame = self.render_jpeg(w, h, bars);
            self.frame_no += 1;
            return Some(frame);
        }
        let fmt = OutFormat::decode(self.reg(REG_FORMAT_CTRL00));
        let bpp = if fmt == OutFormat::Y8 { 1 } else { 2 };
        let mut out = Vec::with_capacity((w * h * bpp) as usize);
        for y in 0..h {
            let mut x = 0;
            while x < w {
                match fmt {
                    OutFormat::Rgb565 { rgb_order } => {
                        let (r, g, b) = self.pixel(x, y, w, h, bars);
                        let (r, b) = if rgb_order { (r, b) } else { (b, r) };
                        let v = (u16::from(r >> 3) << 11)
                            | (u16::from(g >> 2) << 5)
                            | u16::from(b >> 3);
                        out.extend_from_slice(&v.to_be_bytes());
                        x += 1;
                    }
                    OutFormat::Y8 => {
                        let (y8, _, _) = yuv(self.pixel(x, y, w, h, bars));
                        out.push(y8);
                        x += 1;
                    }
                    OutFormat::Yuv422 { seq } => {
                        // One U/V pair per two pixels, taken from the first.
                        let (y0, u, v) = yuv(self.pixel(x, y, w, h, bars));
                        let (y1, _, _) = if x + 1 < w {
                            yuv(self.pixel(x + 1, y, w, h, bars))
                        } else {
                            (y0, u, v)
                        };
                        let quad = match seq {
                            0 => [y0, u, y1, v],
                            1 => [y0, v, y1, u],
                            2 => [u, y0, v, y1],
                            _ => [v, y0, u, y1],
                        };
                        // An odd final pixel carries only its first half.
                        let n = if x + 1 < w { 4 } else { 2 };
                        out.extend_from_slice(&quad[..n]);
                        x += 2;
                    }
                }
            }
        }
        self.frame_no += 1;
        Some(out)
    }

    /// One JPEG frame as it leaves D[7:0]; see the module doc for the sizing.
    fn render_jpeg(&mut self, w: u32, h: u32, bars: bool) -> Vec<u8> {
        let qs = self.reg(REG_COMPRESSION_CTRL07) & 0x3F;
        let quality = (100 - (u32::from(qs) * 3) / 2).clamp(5, 100) as u8;
        let mut jpg = jpeg_baseline::encode_rgb(w, h, quality, |x, y| self.pixel(x, y, w, h, bars));
        if jpg.len() % JPEG_DMA_BLOCK == 0 {
            // COM segment with an empty body: FF FE, length 2.
            jpg.splice(2..2, [0xFF, 0xFE, 0x00, 0x02]);
        }
        let encoded = jpg.len();
        let padded = (encoded / JPEG_DMA_BLOCK + 1) * JPEG_DMA_BLOCK - 1;
        jpg.resize(padded, 0);
        if !self.jpeg_logged {
            self.jpeg_logged = true;
            self.log.push(format!(
                "JPEG {w}x{h} QS={qs} (quality {quality}): {encoded} B, {padded} B on the bus"
            ));
        }
        jpg
    }
}

/// BT.601 full-range RGB → YUV, integer arithmetic.
fn yuv((r, g, b): (u8, u8, u8)) -> (u8, u8, u8) {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let y = (77 * r + 150 * g + 29 * b) >> 8;
    let u = ((-43 * r - 85 * g + 128 * b) >> 8) + 128;
    let v = ((128 * r - 107 * g - 21 * b) >> 8) + 128;
    (
        y.clamp(0, 255) as u8,
        u.clamp(0, 255) as u8,
        v.clamp(0, 255) as u8,
    )
}

/// The sensor: one register file shared by the SCCB slave and the DVP output.
#[derive(Debug)]
pub struct Ov3660 {
    inner: Mutex<Inner>,
}

impl Default for Ov3660 {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::new()),
        }
    }
}

impl Ov3660 {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A poisoned lock means a panic mid-update in this same model; the
        // register file is still the best evidence there is.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Current register value — for tests and inspection.
    pub fn reg(&self, addr: u16) -> u8 {
        self.lock().reg(addr)
    }

    /// The output window the firmware programmed, in pixels.
    pub fn output_size(&self) -> (u32, u32) {
        self.lock().size()
    }

    /// Frames rendered so far.
    pub fn frames_rendered(&self) -> u64 {
        self.lock().frame_no
    }
}

impl DvpSensor for Ov3660 {
    fn next_frame(&self) -> Option<Vec<u8>> {
        self.lock().render()
    }
}

const INPUT_CHANNELS: &[InputChannel] = &[
    InputChannel {
        key: std::borrow::Cow::Borrowed("pattern"),
        label: std::borrow::Cow::Borrowed("Scene (0 bars, 1 checker, 2 gradient, 3 flat)"),
        unit: std::borrow::Cow::Borrowed(""),
        min: 0.0,
        max: 3.0,
        default: Some(0.0),
    },
    InputChannel {
        key: std::borrow::Cow::Borrowed("brightness"),
        label: std::borrow::Cow::Borrowed("Brightness"),
        unit: std::borrow::Cow::Borrowed("%"),
        min: 0.0,
        max: 100.0,
        default: Some(100.0),
    },
    InputChannel {
        key: std::borrow::Cow::Borrowed("motion"),
        label: std::borrow::Cow::Borrowed("Scene motion"),
        unit: std::borrow::Cow::Borrowed("px/frame"),
        min: 0.0,
        max: 64.0,
        default: Some(0.0),
    },
];

/// The SCCB side of the part, as the I²C controller sees it.
#[derive(Debug)]
pub struct Ov3660Sccb {
    address: u8,
    sensor: Arc<Ov3660>,
    component_id: Option<String>,
    /// The handle that owns the part's stimulus and logs. A matrix twin (the
    /// same sensor reached through another controller) carries neither, or
    /// the stimulus walk would find the camera twice and refuse it as
    /// ambiguous.
    primary: bool,
}

impl Ov3660Sccb {
    pub fn new(address: u8, sensor: Arc<Ov3660>) -> Self {
        Self {
            address,
            sensor,
            component_id: None,
            primary: true,
        }
    }

    /// A second SCCB handle on the same sensor, for another I²C controller
    /// that reaches its pads through the GPIO matrix
    /// (`SystemBus::attach_i2c_matrix_twins`).
    pub fn twin(address: u8, sensor: Arc<Ov3660>) -> Self {
        Self {
            primary: false,
            ..Self::new(address, sensor)
        }
    }
}

impl I2cDevice for Ov3660Sccb {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        self.sensor.lock().idx = 0;
    }

    fn stop(&mut self) {
        self.sensor.lock().idx = 0;
    }

    fn write(&mut self, data: u8) {
        let mut s = self.sensor.lock();
        match s.idx {
            0 => s.ptr = (s.ptr & 0x00FF) | (u16::from(data) << 8),
            1 => s.ptr = (s.ptr & 0xFF00) | u16::from(data),
            _ => {
                let p = s.ptr;
                s.write_reg(p, data);
                s.ptr = p.wrapping_add(1);
            }
        }
        s.idx = s.idx.saturating_add(1);
    }

    fn read(&mut self) -> u8 {
        let mut s = self.sensor.lock();
        let p = s.ptr;
        s.ptr = p.wrapping_add(1);
        s.reg(p)
    }

    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        if !self.primary {
            return Vec::new();
        }
        vec![crate::peripheral_log::PeripheralLog::new(
            "sccb",
            self.sensor.lock().log.clone(),
        )]
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        if !self.primary {
            return None;
        }
        Some(self)
    }
}

impl SimInput for Ov3660Sccb {
    fn input_channels(&self) -> &'static [InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        let mut s = self.sensor.lock();
        match key {
            "pattern" => s.scene = Scene::from_value(value),
            "brightness" => s.brightness_pct = value,
            "motion" => s.motion_px = value,
            _ => unreachable!("require_channel validated the key"),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Ov3660Kit;
pub static OV3660_KIT: Ov3660Kit = Ov3660Kit;

static OV3660_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(INPUT_CHANNELS),
    device_type: std::borrow::Cow::Borrowed("ov3660"),
    label: std::borrow::Cow::Borrowed("OV3660 Camera"),
    summary: std::borrow::Cow::Borrowed(
        "OmniVision 3 MP camera: SCCB control at 0x3C plus 8-bit DVP frames into the ESP32-S3 LCD_CAM.",
    ),
    detail: std::borrow::Cow::Borrowed(
        "SCCB register file with 16-bit addressing (chip ID 0x3660, software reset). \
         Frames follow the output window (0x3808-0x380B) and format (0x4300: RGB565, \
         YUV422, Y8) the driver programs, and stream through LCD_CAM's camera slave and \
         GDMA exactly as esp32-camera expects. The scene is synthetic: drive `pattern`, \
         `brightness` and `motion`; the colour-bar test-pattern bit (0x503D[7]) overrides \
         it. JPEG mode (0x3821[5]) emits real baseline JPEG frames at the quality in \
         0x4407. connection: the I2C controller \
         carrying SIOD/SIOC (esp32-camera uses i2c1 by default).",
    ),
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: std::borrow::Cow::Borrowed(&[
        ConfigKey {
            name: std::borrow::Cow::Borrowed("i2c_address"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("7-bit SCCB address. Defaults to 0x3C (fixed on the part)."),
        },
        crate::peripherals::components::supply::POWERED_CONFIG_KEY,
    ]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

impl PeripheralKit for Ov3660Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &OV3660_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(OV3660_SCCB_ADDR)?;
        let sensor = Arc::new(Ov3660::new());
        let powered = crate::peripherals::components::supply::powered_from_config(ctx);
        ctx.attach_i2c_device(Box::new(Ov3660Sccb::new(address, sensor.clone())))?;
        // An unpowered sensor NACKs on SCCB (the supply gate in
        // `attach_i2c_device`) and drives nothing on the DVP pins either, so it
        // is simply not bound to the camera slave.
        if !powered {
            return Ok(());
        }
        // SIOD/SIOC are two pads; on a chip whose I²C controllers reach any pad
        // through the GPIO matrix the camera answers whichever controller the
        // firmware routes there. `esp32-camera` opens I2C1 while the canvas
        // names the controller its pin table offers (I2C0 on the S3), so
        // without twins the probe NACKs. No route, or fixed-pin controllers:
        // nothing is added.
        let connection = ctx.ext.connection.clone();
        let route = ctx.ext.route.clone();
        ctx.bus.attach_i2c_matrix_twins(&connection, &route, || {
            Box::new(Ov3660Sccb::twin(address, sensor.clone()))
        });
        // Bind the DVP output to the chip's camera interface (ESP32-S3:
        // LCD_CAM's camera slave). On a chip without one the SCCB side still
        // works (probe, ID, register I/O) and no frames are produced, which is
        // what a sensor on a board with no camera interface amounts to.
        ctx.bus.attach_dvp_sensor(sensor);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev() -> Ov3660Sccb {
        Ov3660Sccb::new(OV3660_SCCB_ADDR, Arc::new(Ov3660::new()))
    }

    /// `SCCB_Write16`: one transaction, [hi, lo, data].
    fn write16(d: &mut Ov3660Sccb, reg: u16, v: u8) {
        d.start();
        d.write((reg >> 8) as u8);
        d.write(reg as u8);
        d.write(v);
        d.stop();
    }

    /// `SCCB_Read16`: pointer write, STOP, then a one-byte read transaction.
    fn read16(d: &mut Ov3660Sccb, reg: u16) -> u8 {
        d.start();
        d.write((reg >> 8) as u8);
        d.write(reg as u8);
        d.stop();
        d.start();
        let v = d.read();
        d.stop();
        v
    }

    fn configure_qqvga_rgb565(d: &mut Ov3660Sccb) {
        write16(d, REG_X_OUTPUT_SIZE_H, 0);
        write16(d, REG_X_OUTPUT_SIZE_L, 160);
        write16(d, REG_Y_OUTPUT_SIZE_H, 0);
        write16(d, REG_Y_OUTPUT_SIZE_L, 120);
        write16(d, REG_FORMAT_CTRL00, 0x61);
    }

    #[test]
    fn chip_id_reads_0x3660_and_is_read_only() {
        let mut d = dev();
        assert_eq!(read16(&mut d, REG_CHIP_ID_H), 0x36);
        assert_eq!(read16(&mut d, REG_CHIP_ID_L), 0x60);
        write16(&mut d, REG_CHIP_ID_H, 0x12);
        assert_eq!(read16(&mut d, REG_CHIP_ID_H), 0x36);
    }

    #[test]
    fn registers_round_trip_and_pointer_auto_increments() {
        let mut d = dev();
        d.start();
        d.write(0x50);
        d.write(0x00);
        d.write(0xAA);
        d.write(0xBB);
        d.stop();
        assert_eq!(read16(&mut d, 0x5000), 0xAA);
        assert_eq!(read16(&mut d, 0x5001), 0xBB);
    }

    #[test]
    fn software_reset_restores_defaults_and_self_clears() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        write16(&mut d, 0x5000, 0x77);
        write16(&mut d, REG_SYSTEM_CTROL0, 0x82);
        assert_eq!(read16(&mut d, REG_SYSTEM_CTROL0), 0x02);
        assert_eq!(read16(&mut d, 0x5000), 0);
        assert_eq!(d.sensor.output_size(), (MAX_W, MAX_H));
    }

    #[test]
    fn rgb565_frame_follows_programmed_window_and_starts_with_white_bar() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        let f = d.sensor.next_frame().expect("streaming");
        assert_eq!(f.len(), 160 * 120 * 2);
        // First bar is white; RGB565 big-endian on the wire.
        assert_eq!(&f[..2], &[0xFF, 0xFF]);
        // Last pixel of the first line sits in the black bar.
        assert_eq!(&f[159 * 2..160 * 2], &[0x00, 0x00]);
        // Red bar (sixth of eight) encodes as 0xF800.
        let red_x = 160 * 5 / 8 + 2;
        assert_eq!(&f[red_x * 2..red_x * 2 + 2], &[0xF8, 0x00]);
    }

    #[test]
    fn bgr_sequence_swaps_red_and_blue() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        write16(&mut d, REG_FORMAT_CTRL00, 0x60);
        let f = d.sensor.next_frame().unwrap();
        let red_x = 160 * 5 / 8 + 2;
        assert_eq!(&f[red_x * 2..red_x * 2 + 2], &[0x00, 0x1F]);
    }

    #[test]
    fn yuv422_and_y8_frame_sizes() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        write16(&mut d, REG_FORMAT_CTRL00, 0x30);
        let f = d.sensor.next_frame().unwrap();
        assert_eq!(f.len(), 160 * 120 * 2);
        // White: Y=255 (clamped), U=V=128 → Y U Y V.
        assert_eq!(&f[..4], &[255, 128, 255, 128]);
        write16(&mut d, REG_FORMAT_CTRL00, 0x10);
        assert_eq!(d.sensor.next_frame().unwrap().len(), 160 * 120);
    }

    #[test]
    fn power_down_produces_no_frames() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        write16(&mut d, REG_SYSTEM_CTROL0, 0x42);
        assert!(d.sensor.next_frame().is_none());
        write16(&mut d, REG_SYSTEM_CTROL0, 0x02);
        assert!(d.sensor.next_frame().is_some());
    }

    /// What `esp32-camera` does with a JPEG frame: SOI at byte 0 of the first
    /// DMA block, and — in PSRAM mode — EOI inside the last 1 KiB block plus
    /// one byte of the `(EOFs + 1) × 1024` it reports.
    fn assert_driver_finds_the_jpeg(f: &[u8]) -> usize {
        assert_eq!(
            &f[..3],
            &[0xFF, 0xD8, 0xFF],
            "SOI at the start of the first block"
        );
        assert_ne!(f.len() % JPEG_DMA_BLOCK, 0, "never an exact block multiple");
        let reported = (f.len() / JPEG_DMA_BLOCK + 1) * JPEG_DMA_BLOCK;
        let window = reported - (JPEG_DMA_BLOCK + 1);
        let eoi = f[window..]
            .windows(2)
            .position(|w| w == [0xFF, 0xD9])
            .map(|p| p + window)
            .expect("EOI inside the PSRAM probe window");
        assert!(
            f[eoi + 2..].iter().all(|&b| b == 0),
            "only zero padding after EOI"
        );
        eoi + 2
    }

    #[test]
    fn jpeg_mode_emits_a_decodable_frame_the_driver_can_size() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        write16(&mut d, REG_TIMING_TC_REG21, TC21_JPEG_EN);
        write16(&mut d, REG_COMPRESSION_CTRL07, 12);
        let f = d.sensor.next_frame().expect("JPEG frames flow");
        let end = assert_driver_finds_the_jpeg(&f);
        let mut dec = jpeg_decoder::Decoder::new(&f[..end]);
        let px = dec.decode().expect("a baseline decoder accepts the frame");
        let info = dec.info().unwrap();
        assert_eq!(
            (info.width, info.height),
            (160, 120),
            "the programmed window"
        );
        // Colour bars: the first bar is white, the sixth (x = 5/8 w) red.
        let at = |x: usize, y: usize| &px[(y * 160 + x) * 3..(y * 160 + x) * 3 + 3];
        assert!(
            at(4, 60).iter().all(|&c| c > 220),
            "white bar: {:?}",
            at(4, 60)
        );
        let red = at(110, 60);
        assert!(
            red[0] > 200 && red[1] < 60 && red[2] < 60,
            "red bar: {red:?}"
        );
        let log = d.logs().remove(0);
        assert_eq!(log.count_matching("JPEG 160x120 QS=12"), 1, "logged once");
    }

    #[test]
    fn jpeg_quality_register_trades_size_for_fidelity() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        write16(&mut d, REG_TIMING_TC_REG21, TC21_JPEG_EN);
        d.set_input("pattern", 2.0).unwrap(); // gradient: costly to compress
        write16(&mut d, REG_COMPRESSION_CTRL07, 4);
        let fine = assert_driver_finds_the_jpeg(&d.sensor.next_frame().unwrap());
        write16(&mut d, REG_COMPRESSION_CTRL07, 50);
        let coarse = assert_driver_finds_the_jpeg(&d.sensor.next_frame().unwrap());
        assert!(coarse < fine, "QS 50 ({coarse} B) < QS 4 ({fine} B)");
    }

    #[test]
    fn a_jpeg_on_a_block_boundary_is_moved_off_it() {
        // A 94x56 colour-bar window at QS 10 encodes to exactly 2048 bytes:
        // left alone, esp32-camera's PSRAM path could never find its EOI.
        let (w, h) = (94u32, 56u32);
        let bare = jpeg_baseline::encode_rgb(w, h, 85, |x, _| BARS[(x * 8 / w) as usize % 8]);
        assert_eq!(
            bare.len() % JPEG_DMA_BLOCK,
            0,
            "the case under test: {} B",
            bare.len()
        );
        let mut d = dev();
        write16(&mut d, REG_X_OUTPUT_SIZE_H, 0);
        write16(&mut d, REG_X_OUTPUT_SIZE_L, w as u8);
        write16(&mut d, REG_Y_OUTPUT_SIZE_H, 0);
        write16(&mut d, REG_Y_OUTPUT_SIZE_L, h as u8);
        write16(&mut d, REG_TIMING_TC_REG21, TC21_JPEG_EN);
        write16(&mut d, REG_COMPRESSION_CTRL07, 10); // quality 85
        let f = d.sensor.next_frame().unwrap();
        let end = assert_driver_finds_the_jpeg(&f);
        assert_eq!(
            end,
            bare.len() + 4,
            "a 4-byte COM segment moved EOI off the boundary"
        );
        jpeg_decoder::Decoder::new(&f[..end])
            .decode()
            .expect("still decodes with the COM segment");
    }

    #[test]
    fn stimulus_drives_scene_and_test_pattern_overrides_it() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        d.set_input("pattern", 1.0).unwrap();
        let f = d.sensor.next_frame().unwrap();
        // Checkerboard: (16,0) is the first black square.
        assert_eq!(&f[16 * 2..16 * 2 + 2], &[0x00, 0x00]);
        write16(&mut d, REG_PRE_ISP_TEST, PRE_ISP_TEST_EN);
        let f = d.sensor.next_frame().unwrap();
        assert_eq!(&f[16 * 2..16 * 2 + 2], &[0xFF, 0xFF]);
        d.set_input("brightness", 0.0).unwrap();
        let f = d.sensor.next_frame().unwrap();
        assert!(f.iter().all(|&b| b == 0));
        assert!(d.set_input("brightness", 101.0).is_err());
        assert!(d.set_input("nope", 1.0).is_err());
    }

    #[test]
    fn motion_scrolls_the_scene_between_frames() {
        let mut d = dev();
        configure_qqvga_rgb565(&mut d);
        d.set_input("motion", 20.0).unwrap();
        let a = d.sensor.next_frame().unwrap();
        let b = d.sensor.next_frame().unwrap();
        assert_ne!(a, b);
        // Frame 1 is frame 0 shifted left by 20 px.
        assert_eq!(&b[..2], &a[20 * 2..20 * 2 + 2]);
        assert_eq!(d.sensor.frames_rendered(), 2);
    }
}
