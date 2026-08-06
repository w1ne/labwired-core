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
            num_pixels: num_pixels.max(1),
            frame: Vec::new(),
            pixels: Vec::new(),
            component_id: None,
        }
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
        ctx.attach_spi_device(Box::new(Apa102::new(cs, n)))?;
        Ok(())
    }
}
