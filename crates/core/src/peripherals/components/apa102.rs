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

use crate::peripherals::spi::SpiDevice;

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
    /// manifest darkens it — see [`crate::peripherals::components::supply`].
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
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        // Same evidence shape as the WS2812 model: one framebuffer artifact,
        // RGB bytes in pixel order, brightness folded into the meta.
        let flat: Vec<u8> = self.pixels.iter().flat_map(|(rgb, _)| *rgb).collect();
        let brightness: Vec<u8> = self.pixels.iter().map(|(_, b)| *b).collect();
        vec![crate::inspect::Artifact {
            kind: "framebuffer".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "w": self.pixels.len(),
                "h": 1,
                "format": "APA102_RGB",
                "generation": crate::inspect::artifact_generation(&flat),
                "brightness": brightness,
                // Reported so a dark strip explains itself: without it, "no
                // supply" is indistinguishable from "firmware never wrote".
                "powered": self.powered,
                "cs_pin": self.cs_pin,
            }),
            bytes: crate::inspect::artifact_bytes(&flat, opts),
        }]
    }
}

// ---- Kit ----

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Apa102Kit;
pub static APA102_KIT: Apa102Kit = Apa102Kit;

static APA102_METADATA: KitMetadata = KitMetadata {
    device_type: "apa102",
    label: "APA102 DotStar Strip",
    summary: "SPI-clocked addressable RGB LED strip (write-only sink).",
    detail: "APA102/DotStar, 32-bit frames with 5-bit global brightness. \
             Latches on CS release. Power and daisy-chain delay not modelled.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[
        ConfigKey {
            name: "cs_pin",
            ty: ConfigType::Str,
            doc: "Chip-select pin label the firmware drives (e.g. \"PA4\").",
        },
        ConfigKey {
            name: "num_pixels",
            ty: ConfigType::Int,
            doc: "Strip length in LEDs. Defaults to 8.",
        },
        crate::peripherals::components::supply::POWERED_CONFIG_KEY,
    ],
    labs: &[],
    inputs: &[],
};

impl PeripheralKit for Apa102Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &APA102_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let n = ctx.config_i64("num_pixels").unwrap_or(8).max(1) as usize;
        // Supply state. `Some(false)` is the only value that changes anything;
        // an absent key means powered. See `components::supply`.
        ctx.attach_spi_device(Box::new(Apa102::new(cs, n).with_powered(
            crate::peripherals::components::supply::powered_from_config(ctx),
        )))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Supply ───────────────────────────────────────────────────────────
    //
    // THE MEASURED BUG (st7789.rs carries the full account): a diagram wiring
    // only a part's signal pins — no supply, no GND — ran and reported the
    // part working. An APA102 draws every milliamp of LED current from the
    // rail, so a signals-only strip is completely dark on a bench.

    /// One start frame plus two full-brightness LED frames (red, then green),
    /// latched on CS release exactly as a driver sends them.
    fn drive_two_pixels(strip: &mut Apa102) {
        strip.cs_select();
        for b in [0, 0, 0, 0] {
            strip.transfer(b);
        }
        for b in [0xFF, 0x00, 0x00, 0xFF] {
            strip.transfer(b); // brightness 0x1F, B=0, G=0, R=0xFF
        }
        for b in [0xFF, 0x00, 0xFF, 0x00] {
            strip.transfer(b); // B=0, G=0xFF, R=0
        }
        strip.cs_release();
    }

    fn supply_meta(strip: &Apa102) -> serde_json::Value {
        SpiDevice::artifacts(strip, "strip", &crate::inspect::InspectOpts::default())
            .into_iter()
            .next()
            .expect("one framebuffer artifact")
            .meta
    }

    /// The POSITIVE control. Without it, "unpowered is dark" would also pass on
    /// a model that never latches anything.
    #[test]
    fn a_powered_strip_driven_this_way_latches_its_pixels() {
        let mut strip = Apa102::new("PA4", 8);
        drive_two_pixels(&mut strip);

        assert!(strip.powered(), "no supply config at all must mean powered");
        assert_eq!(
            strip.pixels(),
            &[([0xFF, 0, 0], 0x1F), ([0, 0xFF, 0], 0x1F)],
            "red then green at full brightness",
        );
        assert_eq!(supply_meta(&strip)["powered"], true);
    }

    /// The FIX. Identical drive, supply declared absent: nothing latches.
    #[test]
    fn an_unpowered_strip_reports_dark_on_every_field_the_bug_reported() {
        let mut strip = Apa102::new("PA4", 8).with_powered(false);
        drive_two_pixels(&mut strip);

        assert!(strip.pixels().is_empty(), "no supply, no light");
        let m = supply_meta(&strip);
        assert_eq!(m["w"], 0, "an unlit strip reports zero pixels wide");
        assert_eq!(m["brightness"], serde_json::json!([]));
        assert_eq!(m["powered"], false, "the artifact must say WHY it is dark");
    }

    /// Colour must not ACCUMULATE either: the bus is refused, so the latched
    /// pixel list is untouched rather than merely under-reported.
    #[test]
    fn an_unpowered_strip_never_accumulates_colour() {
        let mut strip = Apa102::new("PA4", 8).with_powered(false);
        for _ in 0..5 {
            drive_two_pixels(&mut strip);
        }
        assert!(strip.pixels().is_empty());
    }

    /// ⚠️ THE BACKWARDS-COMPATIBILITY GUARD. Every hand-written lab manifest
    /// declares no supply at all.
    #[test]
    fn absent_supply_information_means_powered() {
        assert!(
            Apa102::new("PA4", 8).powered(),
            "the default must be powered"
        );
        assert!(
            Apa102::new("PA4", 8).with_powered(true).powered(),
            "an explicit true is powered too — only `false` darkens",
        );
    }
}
