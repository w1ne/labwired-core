// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

/// Native panel resolution. The Waveshare 2.9" tri-color module is wired
/// portrait at the silicon level; firmware rotation (handled in the driver)
/// presents it as 296×128 landscape.
const WIDTH: usize = 128;
const HEIGHT: usize = 296;
const WIDTH_BYTES: usize = WIDTH / 8;
const PLANE_BYTES: usize = WIDTH_BYTES * HEIGHT;

/// Protocol state machine. SSD1680 multiplexes command vs data via a D/C
/// GPIO pin in real silicon; the simulator avoids needing a GPIO observer
/// by deriving expected byte counts from the command set itself — the same
/// trick used by the ILI9341 model.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum ProtoState {
    Idle,
    AwaitingParams {
        cmd: u8,
        params: [u8; 4],
        have: u8,
        want: u8,
    },
    StreamingBlack {
        remaining: u32,
    },
    StreamingRed {
        remaining: u32,
    },
}

/// SSD1680 tri-color 2.9" e-paper panel (Waveshare GDEM029C90 / Good Display
/// equivalent). 128×296 native, two 1bpp planes (black + red).
///
/// Models the subset of the SSD1680 command set actually emitted by the
/// `GxEPD2_290_C90c` Arduino driver (15 commands). RAM-X/RAM-Y windowing
/// and counter commands are honored; data-entry mode 0x03 (X+/Y+, X-major)
/// is the only mode used by GxEPD2 and the only one supported here.
///
/// Stream termination on 0x24/0x26: GxEPD2 always pre-configures the window
/// (0x44/0x45) and counter (0x4E/0x4F) before opening a stream, so the byte
/// count is deterministic = (col_end - col_start + 1) * (row_end - row_start + 1).
#[derive(Debug, serde::Serialize)]
pub struct Ssd1680Tricolor290 {
    cs_pin: String,

    // Power / mode flags driven by the SSD1680 command set.
    hibernating: bool,
    power_on: bool,
    /// Set when 0x20 (master activation) has been received with the
    /// "update display" bit pattern in 0x22's parameter. Cleared by the
    /// next stream-write so the UI can detect "refresh-and-flip".
    refresh_pending: bool,
    /// True between 0x12 SWRESET arriving and the first window setup —
    /// purely diagnostic, not used to gate behavior.
    reset_seen: bool,

    // RAM window — values are *byte* coordinates for X, *pixel* coordinates for Y,
    // matching the SSD1680 datasheet (0x44 takes X/8, 0x45 takes raw Y).
    col_start_bytes: u8,
    col_end_bytes: u8,
    row_start: u16,
    row_end: u16,
    cur_col_bytes: u8,
    cur_row: u16,

    /// Black plane: 1 = white (no ink), 0 = black. Row-major, MSB-first within byte.
    /// 4736 bytes for 128×296.
    #[serde(skip_serializing)]
    black_plane: Vec<u8>,
    /// Red plane: 1 = no-red, 0 = red. Stored exactly as received on the wire
    /// (GxEPD2 already inverts source bitmap data before 0x26 — see Display.cpp).
    /// Composition rule in the UI: red dominates black where red bit == 0.
    #[serde(skip_serializing)]
    red_plane: Vec<u8>,

    /// Generation counter incremented every refresh — UI uses it to invalidate
    /// its rendered cache without diffing the planes.
    refresh_generation: u32,

    #[serde(skip_serializing)]
    state: ProtoState,

    /// Data/Command (D/C) GPIO label, if wired (e.g. "GPIO17"). When set, the
    /// bus latches that pin's output level via [`SpiDevice::set_dc_level`]
    /// before each transfer, so framing is driven by the real GPIO exactly
    /// like silicon — no protocol-state inference, no library thunk.
    #[serde(skip)]
    dc_pin: Option<String>,
    /// Latched D/C level (low = command, high = data), pushed by the bus.
    #[serde(skip)]
    dc_level: bool,
    /// Resolved `(GPIO output reg address, bit)` for the D/C line; set by the
    /// bus at attach time so it knows where to sample the level from.
    #[serde(skip)]
    dc_source: Option<(u64, u8)>,
}

impl Default for Ssd1680Tricolor290 {
    fn default() -> Self {
        Self::new("PA4")
    }
}

impl Ssd1680Tricolor290 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            hibernating: false,
            power_on: false,
            refresh_pending: false,
            reset_seen: false,
            col_start_bytes: 0,
            col_end_bytes: (WIDTH_BYTES as u8) - 1,
            row_start: 0,
            row_end: (HEIGHT as u16) - 1,
            cur_col_bytes: 0,
            cur_row: 0,
            // Fresh panel — both planes erased (1 = white / 1 = no-red).
            black_plane: vec![0xFF; PLANE_BYTES],
            red_plane: vec![0xFF; PLANE_BYTES],
            refresh_generation: 0,
            state: ProtoState::Idle,
            dc_pin: None,
            dc_level: false,
            dc_source: None,
        }
    }

    /// Wire a Data/Command GPIO line (e.g. "GPIO17"). With a D/C pin the panel
    /// frames command vs data from the real GPIO level (silicon-accurate);
    /// without one it falls back to protocol-state inference.
    pub fn with_dc_pin(mut self, dc_pin: impl Into<String>) -> Self {
        self.dc_pin = Some(dc_pin.into());
        self
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }

    pub fn black_plane(&self) -> &[u8] {
        &self.black_plane
    }

    pub fn red_plane(&self) -> &[u8] {
        &self.red_plane
    }

    pub fn refresh_generation(&self) -> u32 {
        self.refresh_generation
    }

    pub fn power_on(&self) -> bool {
        self.power_on
    }

    /// Process one command byte (DC=low on real hardware). Mirrors
    /// [`Uc8151dTricolor290::command_byte`] so the ESP32/GxEPD2 thunk path —
    /// which knows the DC line from the calling function identity — can drive
    /// the panel without going through the SPI peripheral. Drives the same
    /// datasheet dispatch the real SPI `transfer()` path uses.
    pub fn command_byte(&mut self, cmd: u8) {
        self.handle_command(cmd);
    }

    /// Process one data byte (DC=high on real hardware). Routes to the active
    /// param accumulator or pixel-plane stream set up by the last command.
    /// Spurious data with no active command is ignored.
    pub fn data_byte(&mut self, byte: u8) {
        match self.state {
            ProtoState::AwaitingParams {
                cmd,
                mut params,
                mut have,
                want,
            } => {
                params[have as usize] = byte;
                have += 1;
                if have >= want {
                    self.handle_params_complete(cmd, &params);
                    self.state = ProtoState::Idle;
                } else {
                    self.state = ProtoState::AwaitingParams {
                        cmd,
                        params,
                        have,
                        want,
                    };
                }
            }
            ProtoState::StreamingBlack { remaining } => {
                self.write_plane_byte(PlaneKind::Black, byte);
                let left = remaining.saturating_sub(1);
                self.state = if left == 0 {
                    ProtoState::Idle
                } else {
                    ProtoState::StreamingBlack { remaining: left }
                };
            }
            ProtoState::StreamingRed { remaining } => {
                self.write_plane_byte(PlaneKind::Red, byte);
                let left = remaining.saturating_sub(1);
                self.state = if left == 0 {
                    ProtoState::Idle
                } else {
                    ProtoState::StreamingRed { remaining: left }
                };
            }
            ProtoState::Idle => {
                // Data byte with no active command — nothing to consume.
            }
        }
    }

    // ---- Command dispatch ----

    fn handle_command(&mut self, cmd: u8) {
        match cmd {
            0x12 => {
                // SWRESET — software reset. Wipes window/counters but NOT the
                // framebuffer (real silicon preserves RAM; GxEPD2 explicitly
                // clears it via 0x24/0x26 in writeScreenBuffer).
                self.col_start_bytes = 0;
                self.col_end_bytes = (WIDTH_BYTES as u8) - 1;
                self.row_start = 0;
                self.row_end = (HEIGHT as u16) - 1;
                self.cur_col_bytes = 0;
                self.cur_row = 0;
                self.reset_seen = true;
                self.hibernating = false;
                self.state = ProtoState::Idle;
            }
            0x10 => self.await_params(cmd, 1), // Deep sleep (param 0x01 = enter)
            0x11 => self.await_params(cmd, 1), // Data entry mode
            0x18 => self.await_params(cmd, 1), // Temp sensor select
            0x3C => self.await_params(cmd, 1), // Border waveform
            0x21 => self.await_params(cmd, 2), // Display update ctrl 1
            0x22 => self.await_params(cmd, 1), // Display update ctrl 2 (sequence selector)
            0x01 => self.await_params(cmd, 3), // Driver output control (MUX/GD/SM)
            0x44 => self.await_params(cmd, 2), // RAM-X window: start/8, end/8
            0x45 => self.await_params(cmd, 4), // RAM-Y window: start_lo/hi, end_lo/hi
            0x4E => self.await_params(cmd, 1), // RAM-X address counter
            0x4F => self.await_params(cmd, 2), // RAM-Y address counter: lo/hi
            0x20 => {
                // Master activation — kicks the sequence configured by the
                // last 0x22 parameter. We don't model the LUT distinction;
                // any 0x20 after a stream write is treated as a refresh.
                self.refresh_pending = true;
                self.refresh_generation = self.refresh_generation.wrapping_add(1);
                self.state = ProtoState::Idle;
            }
            0x24 => {
                // Write black RAM — open a pixel stream sized to the window.
                let bytes = self.window_byte_count();
                self.cur_col_bytes = self.col_start_bytes;
                self.cur_row = self.row_start;
                self.state = ProtoState::StreamingBlack { remaining: bytes };
            }
            0x26 => {
                // Write red RAM — same window-sized stream.
                let bytes = self.window_byte_count();
                self.cur_col_bytes = self.col_start_bytes;
                self.cur_row = self.row_start;
                self.state = ProtoState::StreamingRed { remaining: bytes };
            }
            _ => {
                // Unknown command — treat as zero-parameter no-op rather than
                // mis-consuming the next byte as a param.
                self.state = ProtoState::Idle;
            }
        }
    }

    fn await_params(&mut self, cmd: u8, want: u8) {
        self.state = ProtoState::AwaitingParams {
            cmd,
            params: [0; 4],
            have: 0,
            want,
        };
    }

    fn handle_params_complete(&mut self, cmd: u8, params: &[u8; 4]) {
        match cmd {
            0x10 if params[0] & 0x01 != 0 => {
                self.hibernating = true;
                self.power_on = false;
            }
            0x22 => {
                // 0xF8 = power-on-only (GxEPD2 _PowerOn), 0x83 = power-off
                // (GxEPD2 _PowerOff), 0xF7 = full update sequence. The
                // following 0x20 is what actually activates.
                match params[0] {
                    0xF8 => self.power_on = true,
                    0x83 => self.power_on = false,
                    _ => {}
                }
            }
            0x44 => {
                self.col_start_bytes = (params[0] & 0x3F).min((WIDTH_BYTES as u8) - 1);
                self.col_end_bytes = (params[1] & 0x3F).min((WIDTH_BYTES as u8) - 1);
            }
            0x45 => {
                let start = ((params[1] as u16) << 8) | (params[0] as u16);
                let end = ((params[3] as u16) << 8) | (params[2] as u16);
                self.row_start = start.min((HEIGHT as u16) - 1);
                self.row_end = end.min((HEIGHT as u16) - 1);
            }
            0x4E => {
                self.cur_col_bytes = (params[0] & 0x3F).min((WIDTH_BYTES as u8) - 1);
            }
            0x4F => {
                let v = ((params[1] as u16) << 8) | (params[0] as u16);
                self.cur_row = v.min((HEIGHT as u16) - 1);
            }
            // 0x01 / 0x11 / 0x18 / 0x3C / 0x21 — params consumed, behavior
            // not modeled because nothing downstream depends on them.
            _ => {}
        }
    }

    fn window_byte_count(&self) -> u32 {
        let w_bytes = self
            .col_end_bytes
            .saturating_sub(self.col_start_bytes)
            .saturating_add(1) as u32;
        let h = self
            .row_end
            .saturating_sub(self.row_start)
            .saturating_add(1) as u32;
        w_bytes * h
    }

    fn write_plane_byte(&mut self, plane: PlaneKind, byte: u8) {
        let idx = (self.cur_row as usize) * WIDTH_BYTES + (self.cur_col_bytes as usize);
        if idx < PLANE_BYTES {
            match plane {
                PlaneKind::Black => self.black_plane[idx] = byte,
                PlaneKind::Red => self.red_plane[idx] = byte,
            }
        }
        self.advance_counter();
    }

    fn advance_counter(&mut self) {
        // Data entry mode 0x03 (X-major, both auto-incrementing) — the only
        // mode GxEPD2 sets. Advance X first; when X passes col_end, wrap to
        // col_start and bump Y (wrapping within the row window).
        if self.cur_col_bytes >= self.col_end_bytes {
            self.cur_col_bytes = self.col_start_bytes;
            if self.cur_row >= self.row_end {
                self.cur_row = self.row_start;
            } else {
                self.cur_row += 1;
            }
        } else {
            self.cur_col_bytes += 1;
        }
    }
}

#[derive(Clone, Copy)]
enum PlaneKind {
    Black,
    Red,
}

/// Wire-format snapshot. Captures everything we need to resume rendering
/// from a pre-warmed boot — both planes, the protocol state machine, the
/// power/refresh flags, and the RAM-window counters.
#[derive(serde::Serialize, serde::Deserialize)]
struct Ssd1680Snap {
    cs_pin: String,
    hibernating: bool,
    power_on: bool,
    refresh_pending: bool,
    reset_seen: bool,
    col_start_bytes: u8,
    col_end_bytes: u8,
    row_start: u16,
    row_end: u16,
    cur_col_bytes: u8,
    cur_row: u16,
    black_plane: Vec<u8>,
    red_plane: Vec<u8>,
    refresh_generation: u32,
    state: ProtoState,
}

impl SpiDevice for Ssd1680Tricolor290 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let snap = Ssd1680Snap {
            cs_pin: self.cs_pin.clone(),
            hibernating: self.hibernating,
            power_on: self.power_on,
            refresh_pending: self.refresh_pending,
            reset_seen: self.reset_seen,
            col_start_bytes: self.col_start_bytes,
            col_end_bytes: self.col_end_bytes,
            row_start: self.row_start,
            row_end: self.row_end,
            cur_col_bytes: self.cur_col_bytes,
            cur_row: self.cur_row,
            black_plane: self.black_plane.clone(),
            red_plane: self.red_plane.clone(),
            refresh_generation: self.refresh_generation,
            state: self.state,
        };
        bincode::serialize(&snap).expect("bincode serialize Ssd1680Snap")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        let snap: Ssd1680Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Ssd1680 snapshot decode: {e}"))
        })?;
        self.cs_pin = snap.cs_pin;
        self.hibernating = snap.hibernating;
        self.power_on = snap.power_on;
        self.refresh_pending = snap.refresh_pending;
        self.reset_seen = snap.reset_seen;
        self.col_start_bytes = snap.col_start_bytes;
        self.col_end_bytes = snap.col_end_bytes;
        self.row_start = snap.row_start;
        self.row_end = snap.row_end;
        self.cur_col_bytes = snap.cur_col_bytes;
        self.cur_row = snap.cur_row;
        if snap.black_plane.len() != self.black_plane.len()
            || snap.red_plane.len() != self.red_plane.len()
        {
            return Err(crate::SimulationError::NotImplemented(format!(
                "Ssd1680 snapshot plane size mismatch: black {} vs {}, red {} vs {}",
                snap.black_plane.len(),
                self.black_plane.len(),
                snap.red_plane.len(),
                self.red_plane.len()
            )));
        }
        self.black_plane = snap.black_plane;
        self.red_plane = snap.red_plane;
        self.refresh_generation = snap.refresh_generation;
        self.state = snap.state;
        Ok(())
    }

    fn cs_select(&mut self) {
        // Each CS-low burst resets the protocol parser. Mid-stream CS-toggling
        // by firmware would otherwise corrupt the next command — though GxEPD2
        // holds CS for entire command+data sequences, so in practice this is
        // a belt-and-braces guard.
        self.state = ProtoState::Idle;
    }

    fn cs_release(&mut self) {
        // Preserve state — a long pixel stream may span the entire CS-low
        // window and we don't want CS-high to discard the framebuffer in
        // flight (next cs_select() resets to Idle for the following command).
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // Silicon-accurate framing when a D/C line is wired: the bus has
        // latched the real GPIO level (low = command, high = data) before
        // this transfer. With no D/C pin (e.g. the STM32 lab), fall back to
        // protocol-state inference: a byte in Idle is a command, otherwise a
        // param/stream byte for the command in flight.
        if self.dc_source.is_some() {
            if self.dc_level {
                self.data_byte(mosi);
            } else {
                self.command_byte(mosi);
            }
        } else {
            // CHEAT(INFER): no D/C line wired — guess command vs data from
            // protocol state — real: sample the D/C GPIO. See FIDELITY.md §E.
            if matches!(self.state, ProtoState::Idle) {
                self.command_byte(mosi);
            } else {
                self.data_byte(mosi);
            }
        }
        // Tri-color e-paper is write-only over SPI (BUSY is a sideband GPIO,
        // not MISO). Return 0 so the bus broadcaster doesn't see us as a
        // MISO source if other devices share the bus.
        0
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

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Ssd1680Tricolor290Kit;
pub static SSD1680_TRICOLOR_290_KIT: Ssd1680Tricolor290Kit = Ssd1680Tricolor290Kit;

static SSD1680_TRICOLOR_290_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "ssd1680_tricolor_290",
    label: "SSD1680 Tri-Color E-Paper",
    summary: "2.9\" tri-color (black/white/red) SSD1680 e-paper over SPI.",
    detail: "Full SPI command + display-data RAM state machine, validated by the e2e e-paper \
             integration test. Same model drives both the STM32F103 lab and the ESP32 \
             playground board.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "Chip-select GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
    }],
    labs: &[
        LabRef {
            board_id: "epaper-tricolor-lab",
            chip: "stm32f103",
            example_dir: "epaper-tricolor-lab",
            demo_elf: "demo-epaper-tricolor-lab.elf",
        },
        LabRef {
            board_id: "esp32-epaper-lab",
            chip: "esp32",
            example_dir: "esp32-epaper-lab",
            demo_elf: "demo-esp32-epaper-lab.elf",
        },
    ],
};

impl PeripheralKit for Ssd1680Tricolor290Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SSD1680_TRICOLOR_290_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut panel = Ssd1680Tricolor290::new(cs_pin);
        if let Some(dc_pin) = ctx.config_str("dc_pin") {
            let (odr_addr, bit) = ctx.resolve_pin_odr(dc_pin).ok_or_else(|| {
                anyhow::anyhow!(
                    "ssd1680_tricolor_290 '{}' dc_pin '{}' could not be resolved to a GPIO output",
                    ctx.device_id(),
                    dc_pin
                )
            })?;
            panel = panel.with_dc_pin(dc_pin.to_string());
            crate::peripherals::spi::SpiDevice::set_dc_source(&mut panel, odr_addr, bit);
        }
        ctx.attach_spi_device(Box::new(panel))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send_seq(dev: &mut Ssd1680Tricolor290, bytes: &[u8]) {
        dev.cs_select();
        for &b in bytes {
            dev.transfer(b);
        }
        dev.cs_release();
    }

    /// Build the exact byte sequence emitted by GxEPD2_290_C90c::_InitDisplay().
    fn init_display_bytes() -> Vec<u8> {
        vec![
            0x12, // SWRESET
            0x01, 0x27, 0x01, 0x00, // Driver output control
            0x11, 0x03, // Data entry mode
            0x3C, 0x05, // Border waveform
            0x18, 0x80, // Temp sensor select
            0x21, 0x00, 0x80, // Display update ctrl 1
            // _setPartialRamArea(0, 0, WIDTH=128, HEIGHT=296)
            0x44, 0x00, 0x0F, // RAM-X window: 0..15 (bytes)
            0x45, 0x00, 0x00, 0x27, 0x01, // RAM-Y window: 0..295
            0x4E, 0x00, // RAM-X counter
            0x4F, 0x00, 0x00, // RAM-Y counter
        ]
    }

    #[test]
    fn init_sequence_sets_window_and_counters() {
        let mut dev = Ssd1680Tricolor290::new("PA4");
        send_seq(&mut dev, &init_display_bytes());

        assert_eq!(dev.col_start_bytes, 0);
        assert_eq!(dev.col_end_bytes, 15);
        assert_eq!(dev.row_start, 0);
        assert_eq!(dev.row_end, 295);
        assert_eq!(dev.cur_col_bytes, 0);
        assert_eq!(dev.cur_row, 0);
    }

    #[test]
    fn clearscreen_fills_both_planes_white() {
        // Replicates GxEPD2_290_C90c::clearScreen(0xFF, 0xFF):
        // init + 0x24 + 4736 bytes of 0xFF + 0x26 + 4736 bytes of ~0xFF=0x00.
        // After: black plane = 0xFF (all white), red plane = 0x00 (all red).
        let mut dev = Ssd1680Tricolor290::new("PA4");
        send_seq(&mut dev, &init_display_bytes());

        dev.cs_select();
        dev.transfer(0x24);
        for _ in 0..PLANE_BYTES {
            dev.transfer(0xFF);
        }
        dev.transfer(0x26);
        for _ in 0..PLANE_BYTES {
            dev.transfer(0x00); // GxEPD2 writes ~color_value, 0xFF → 0x00
        }
        // _Update_Part sequence
        dev.transfer(0x22);
        dev.transfer(0xF7);
        dev.transfer(0x20);
        dev.cs_release();

        assert!(
            dev.black_plane().iter().all(|&b| b == 0xFF),
            "black plane all-white"
        );
        assert!(
            dev.red_plane().iter().all(|&b| b == 0x00),
            "red plane all-red on wire"
        );
        assert!(dev.refresh_pending, "0x20 must arm refresh");
        assert_eq!(dev.refresh_generation, 1);
    }

    #[test]
    fn stream_length_terminates_correctly() {
        // Confirms that exactly PLANE_BYTES bytes are consumed by 0x24,
        // and the very next byte is parsed as a command again.
        let mut dev = Ssd1680Tricolor290::new("PA4");
        send_seq(&mut dev, &init_display_bytes());

        dev.cs_select();
        dev.transfer(0x24);
        for _ in 0..PLANE_BYTES {
            dev.transfer(0xAA);
        }
        // Next byte: should be parsed as command, not pixel.
        dev.transfer(0x12); // SWRESET
        dev.cs_release();
        assert!(
            dev.reset_seen,
            "byte after full stream should be parsed as command"
        );
    }

    #[test]
    fn partial_window_streams_correct_byte_count() {
        // 16×16-pixel window in the top-left corner: 2 bytes × 16 rows = 32 bytes.
        let mut dev = Ssd1680Tricolor290::new("PA4");
        send_seq(
            &mut dev,
            &[
                0x44, 0x00, 0x01, // X: 0..1 (bytes)
                0x45, 0x00, 0x00, 0x0F, 0x00, // Y: 0..15
                0x4E, 0x00, //
                0x4F, 0x00, 0x00, //
            ],
        );

        dev.cs_select();
        dev.transfer(0x24);
        for _ in 0..32 {
            dev.transfer(0x55);
        }
        // 33rd byte after 0x24 must be treated as a command.
        dev.transfer(0x12);
        dev.cs_release();
        assert!(dev.reset_seen);
        // Top-left 2×16 region of black plane should be 0x55, rest untouched.
        for row in 0..16 {
            assert_eq!(dev.black_plane()[row * WIDTH_BYTES], 0x55);
            assert_eq!(dev.black_plane()[row * WIDTH_BYTES + 1], 0x55);
            assert_eq!(
                dev.black_plane()[row * WIDTH_BYTES + 2],
                0xFF,
                "outside window"
            );
        }
    }

    #[test]
    fn power_state_tracks_0x22_param() {
        let mut dev = Ssd1680Tricolor290::new("PA4");
        // _PowerOn: 0x22 0xF8 0x20
        send_seq(&mut dev, &[0x22, 0xF8, 0x20]);
        assert!(dev.power_on);
        // _PowerOff: 0x22 0x83 0x20
        send_seq(&mut dev, &[0x22, 0x83, 0x20]);
        assert!(!dev.power_on);
    }

    #[test]
    fn hibernate_command_sets_flag() {
        let mut dev = Ssd1680Tricolor290::new("PA4");
        send_seq(&mut dev, &[0x10, 0x01]);
        assert!(dev.hibernating);
    }

    #[test]
    fn refresh_generation_advances_per_activation() {
        let mut dev = Ssd1680Tricolor290::new("PA4");
        assert_eq!(dev.refresh_generation, 0);
        send_seq(&mut dev, &[0x20]);
        assert_eq!(dev.refresh_generation, 1);
        send_seq(&mut dev, &[0x20]);
        assert_eq!(dev.refresh_generation, 2);
    }
}
