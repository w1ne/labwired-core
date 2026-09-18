// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::peripherals::spi::SpiDevice;
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

    /// Whether the module's supply pins (VCC, GND) are connected in the design.
    ///
    /// ⚠️ NOT `power_on` below. `power_on` is the SSD1680's own booster/power
    /// sequence, driven by 0x22/0x20 over a bus that works; this is whether the
    /// module has a rail at all. A diagram wiring only CLK/DIN/CS/DC/RST/BUSY
    /// used to run the whole GxEPD2 init and report inked planes and a bumped
    /// `refresh_generation`; on a bench the glass never changes.
    ///
    /// ⚠️ DEFAULTS TO `true`. Only an explicit `powered: false` in the compiled
    /// manifest darkens it — see `peripherals::components::supply`.
    powered: bool,

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
            // Absent supply information means "powered" — see the field's note.
            powered: true,
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

    /// The controller's own power sequence **and** a supply. `transfer`
    /// already refuses the bus when unpowered, so the inner flag can never be
    /// true here; the `&&` is the guard that survives someone later adding
    /// another way to set it.
    pub fn power_on(&self) -> bool {
        self.powered && self.power_on
    }

    /// Declare whether the module's supply is connected. See the `powered`
    /// field. Only ever called with `false`, from `attach`, when the compiled
    /// manifest explicitly says the supply pins are on no net.
    pub fn with_powered(mut self, powered: bool) -> Self {
        self.powered = powered;
        self
    }

    /// True when the module has a supply. See the `powered` field.
    pub fn powered(&self) -> bool {
        self.powered
    }

    /// Process one command byte (DC=low on real hardware). Mirrors
    /// `Uc8151dTricolor290::command_byte` so the ESP32/GxEPD2 thunk path —
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
    /// A tri-color e-paper is not one framebuffer, so it is not reported as
    /// one. It has TWO independent 1-bpp planes and a refresh that decides
    /// whether either is on the glass, and all three facts are evidence:
    ///
    /// * The planes are erased to `0xFF` — the panel's own convention, where a
    ///   set bit is "no ink" — so an inked cell is a byte that is NOT `0xFF`.
    ///   That is the same count the CLI's `black-plane non-FF bytes=` line
    ///   prints, so the two agree by construction.
    /// * `refresh_generation` is the only thing that distinguishes "RAM was
    ///   written" from "the image is on the glass". `labwired_verify`'s
    ///   `min_refresh_generation` clause resolves against it and was
    ///   unreachable for every e-paper until this existed.
    /// * `bytes` carries the black plane followed by the red plane, with
    ///   `plane_bytes` giving the split, because one payload field cannot hold
    ///   two planes and inventing a composite image would be synthesizing a
    ///   picture the model never produced.
    fn artifacts(
        &self,
        id: &str,
        opts: &labwired_core::inspect::InspectOpts,
    ) -> Vec<labwired_core::inspect::Artifact> {
        let (w, h) = self.dimensions();
        let black = self.black_plane();
        let red = self.red_plane();
        let ink = |plane: &[u8]| plane.iter().filter(|&&b| b != 0xFF).count();
        let mut both = Vec::with_capacity(black.len() + red.len());
        both.extend_from_slice(black);
        both.extend_from_slice(red);
        vec![labwired_core::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": w,
                "h": h,
                "format": labwired_core::inspect::artifact_format::EPAPER_TRICOLOR_PLANES,
                "generation": labwired_core::inspect::artifact_generation(&both),
                "plane_bytes": black.len(),
                "black_ink_bytes": ink(black),
                "red_ink_bytes": ink(red),
                "refresh_generation": self.refresh_generation(),
                "power_on": self.power_on(),
                // Reported so a blank panel explains itself: without it, "no
                // supply" is indistinguishable from "firmware never refreshed".
                "powered": self.powered,
            }),
            bytes: labwired_core::inspect::artifact_bytes(&both, opts),
        }]
    }

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

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> labwired_core::SimResult<()> {
        let snap: Ssd1680Snap = bincode::deserialize(bytes).map_err(|e| {
            labwired_core::SimulationError::NotImplemented(format!("Ssd1680 snapshot decode: {e}"))
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
            return Err(labwired_core::SimulationError::NotImplemented(format!(
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
        // THE ONE GATE THAT MAKES AN UNPOWERED PANEL BEHAVE LIKE ONE. Every
        // state change this model has — the RAM window, both plane streams and
        // the 0x20 master activation that bumps `refresh_generation` — arrives
        // through `transfer`. Refusing the bus here leaves the planes erased
        // and the refresh counter at zero by construction rather than masking
        // them at report time. See the `powered` field.
        //
        // BUSY is deliberately still driven to its idle level at attach (see
        // `attach`). On a bench an unpowered panel leaves BUSY floating and
        // GxEPD2 spins to its 30 s escape timeout — which at simulated speed is
        // ~10^7 steps per refresh and reads to a user as a hung firmware, not
        // as a wiring fault. A blank panel that still returns is the honest
        // half we can actually show; the wiring fault itself is reported at
        // design time by the PWR_SUPPLY_UNCONNECTED ERC warning.
        if !self.powered {
            return 0;
        }
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
