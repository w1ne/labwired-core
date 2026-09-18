// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! APA102 / DotStar addressable LED strip — write-only SPI sink.
//! Frame format: 32-bit start frame (0x00000000), then per-LED frames
//! `[0xE0|brightness5, blue, green, red]` in wire order, then enough end-frame
//! clocks. The strip latches on CS release; anything shorter than a start
//! frame plus one LED frame leaves the previous pixels untouched (a glitchy
//! transaction must not blank the strip). MISO is never driven.
//! Out of scope (matches Simulator86): power draw and daisy-chain
//! propagation delay.

use labwired_core::peripherals::spi::SpiDevice;

/// One latched pixel: `[r, g, b]` plus the 5-bit global brightness.
pub type Pixel = ([u8; 3], u8);

#[derive(Debug)]
pub struct Apa102 {
    cs_pin: String,
    /// Whether the strip's supply pins (5V, GND) are connected in the design.
    ///
    /// An APA102 strip is the clearest case of the bug this closes: it draws
    /// its LED current from the rail, not from the data lines, so a diagram
    /// wiring only CLK/DATA/CS produces a strip that on a bench emits nothing
    /// at all while the twin happily reported latched pixel colours.
    ///
    /// ⚠️ DEFAULTS TO `true`. Only an explicit `powered: false` in the compiled
    /// manifest darkens it — see `peripherals::components::supply`.
    powered: bool,
    num_pixels: usize,
    /// Bytes of the in-flight transaction.
    frame: Vec<u8>,
    pixels: Vec<Pixel>,
    component_id: Option<String>,
}

impl Apa102 {
    pub fn new(cs_pin: impl Into<String>, num_pixels: usize) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            // Absent supply information means "powered" — see the field's note.
            powered: true,
            num_pixels: num_pixels.max(1),
            frame: Vec::new(),
            pixels: Vec::new(),
            component_id: None,
        }
    }

    /// Declare whether the strip's supply is connected. See the `powered`
    /// field. Only ever called with `false`, from `attach`, when the compiled
    /// manifest explicitly says the supply pins are on no net.
    pub fn with_powered(mut self, powered: bool) -> Self {
        self.powered = powered;
        self
    }

    /// True when the strip has a supply. See the `powered` field.
    pub fn powered(&self) -> bool {
        self.powered
    }

    pub fn with_component_id(mut self, id: impl Into<String>) -> Self {
        self.component_id = Some(id.into());
        self
    }

    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    pub fn num_pixels(&self) -> usize {
        self.num_pixels
    }

    pub fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    /// Parse a completed transaction. Requires the 4-byte start frame and at
    /// least one well-formed LED frame; anything else leaves the previous
    /// pixels untouched.
    fn latch(&mut self) {
        let f = &self.frame;
        if f.len() < 8 || f[0..4] != [0, 0, 0, 0] {
            return;
        }
        let mut out: Vec<Pixel> = Vec::new();
        for chunk in f[4..].chunks_exact(4) {
            if out.len() >= self.num_pixels {
                break;
            }
            let header = chunk[0];
            if header & 0xE0 != 0xE0 {
                break; // end frame or garbage — stop decoding
            }
            let brightness = header & 0x1F;
            let (b, g, r) = (chunk[1], chunk[2], chunk[3]);
            out.push(([r, g, b], brightness));
        }
        if !out.is_empty() {
            self.pixels = out;
        }
    }
}

impl SpiDevice for Apa102 {
    fn cs_select(&mut self) {
        self.frame.clear();
    }

    fn cs_release(&mut self) {
        self.latch();
    }

    fn transfer(&mut self, mosi_byte: u8) -> u8 {
        // THE ONE GATE THAT MAKES AN UNPOWERED STRIP BEHAVE LIKE ONE. Every
        // byte of every frame arrives here, and `latch` (on CS release) only
        // ever reads what this pushed — so refusing the bus leaves `pixels`
        // empty by construction rather than masking the colours at report time.
        if !self.powered {
            return 0x00;
        }
        self.frame.push(mosi_byte);
        0x00 // MISO not driven
    }

    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn artifacts(
        &self,
        id: &str,
        opts: &labwired_core::inspect::InspectOpts,
    ) -> Vec<labwired_core::inspect::Artifact> {
        // Same evidence shape as the WS2812 model: one framebuffer artifact,
        // RGB bytes in pixel order, brightness folded into the meta.
        let flat: Vec<u8> = self.pixels.iter().flat_map(|(rgb, _)| *rgb).collect();
        let brightness: Vec<u8> = self.pixels.iter().map(|(_, b)| *b).collect();
        vec![labwired_core::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": self.pixels.len(),
                "h": 1,
                "format": "APA102_RGB",
                "generation": labwired_core::inspect::artifact_generation(&flat),
                "brightness": brightness,
                // Reported so a dark strip explains itself: without it, "no
                // supply" is indistinguishable from "firmware never wrote".
                "powered": self.powered,
                "cs_pin": self.cs_pin,
            }),
            bytes: labwired_core::inspect::artifact_bytes(&flat, opts),
        }]
    }
}
