// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Rendering a declared [`artifact:`](labwired_config::ArtifactSpec).**
//!
//! What this is for
//! ================
//! A part that SHOWS something used to need a hand-written Rust model, for one
//! reason: `evidence()` / `artifacts()` had to be implemented, and only a
//! concrete type can implement a trait. So a TM1637 could be simulated by a
//! rule list perfectly and inspect as NOTHING — no text, no panel, no evidence
//! — which is why #1176 named "a `gpio_device` publishes no artifact today" as
//! the thing blocking both segment-display ports.
//!
//! This is the other half. The rules fill a RAM (vars, or a FIFO); the
//! declaration says how to render it; this module does the rendering, once, for
//! every primitive that can carry the key. `declarative_gpio` publishes through
//! [`BusResidentDevice::evidence`](crate::bus::BusResidentDevice::evidence) and
//! `declarative_spi` through
//! [`SpiDevice::artifacts`](crate::peripherals::spi::SpiDevice::artifacts) —
//! two doors, ONE renderer, because two renderers over one declaration is how
//! the same descriptor comes to paint differently on two transports.
//!
//! The shape is not negotiable
//! ===========================
//! ⚠️ The output must be byte-for-byte what the hand-written models published:
//! the browser, the wasm accessors and `panel_artifact_evidence` are already
//! written against `kind` / `id` / `meta.format` / `meta.generation` and the
//! per-part `meta` keys. That is what the migration-parity tests hold — the
//! deleted model kept verbatim as an oracle, the same stimulus through both,
//! every artifact byte and every `meta` field compared.
//!
//! What is declared and what is derived
//! ====================================
//! `meta` entries come two ways, and the split is not stylistic:
//!
//! * `value:` — an EXPRESSION over the part's own state, in the same vocabulary
//!   a rule has. This is nearly everything: `display_on`, `brightness`,
//!   `colon`, `segments`.
//! * `source:` — an engine-derived quantity over the rendered RAM, for the few
//!   facts the expression language genuinely cannot reach because it has no
//!   bit-counting: `lit_bits` (the TM1637's `lit_segments`), `ink_bytes`,
//!   `bytes`.
//!
//! Adding `popcount()` to the expression language instead was considered and
//! refused: it would be one operator that exists for one meta field, in a
//! grammar whose whole argument is that every name in it is a thing a datasheet
//! says.

use anyhow::{Context, Result};
use labwired_config::{expr::Expr, ArtifactFont, ArtifactMetaType, ArtifactSpec, DeviceDescriptor};

use super::rule_machine::{RuleCtx, RuleMachine};
use super::seven_seg_font;
use crate::inspect::{artifact_bytes, artifact_generation, Artifact, InspectOpts};

/// A declared artifact with every expression parsed once, at load.
#[derive(Debug)]
pub struct CompiledArtifact {
    kind: String,
    id_suffix: Option<String>,
    format: Option<String>,
    /// RAM source: variable names in artifact byte order …
    ram_vars: Vec<String>,
    /// … or one FIFO, oldest entry first. Exactly one of the two is non-empty;
    /// [`ArtifactSpec::validate`] is what makes that true.
    ram_fifo: Option<String>,
    font: ArtifactFont,
    digits: Option<usize>,
    meta: Vec<CompiledMeta>,
    bytes: bool,
}

#[derive(Debug)]
struct CompiledMeta {
    key: String,
    value: Option<Expr>,
    source: Option<String>,
    ty: ArtifactMetaType,
}

impl CompiledArtifact {
    /// Compile the `artifact:` block of a descriptor, if it declares one.
    ///
    /// Returns `Ok(None)` for a part that shows nothing, which is every
    /// descriptor written before the key existed — so a part with no
    /// declaration allocates nothing and publishes nothing, rather than
    /// publishing an empty panel that looks like a working one.
    pub fn from_descriptor(desc: &DeviceDescriptor) -> Result<Option<Self>> {
        let Some(spec) = &desc.behavior.artifact else {
            return Ok(None);
        };
        Self::compile(spec, &desc.r#type).map(Some)
    }

    fn compile(spec: &ArtifactSpec, part: &str) -> Result<Self> {
        spec.validate(part)?;
        let meta = spec
            .meta
            .iter()
            .map(|f| {
                Ok(CompiledMeta {
                    key: f.key.clone(),
                    value: match &f.value {
                        Some(src) => Some(Expr::parse(src).map_err(|e| anyhow::anyhow!("{e}"))?),
                        None => None,
                    },
                    source: f.source.clone(),
                    ty: f.r#type,
                })
            })
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("part '{part}' has an invalid `artifact.meta` expression"))?;
        Ok(Self {
            kind: spec.kind.clone(),
            id_suffix: spec.id.clone(),
            format: spec.format.clone(),
            ram_vars: spec.ram.vars.clone(),
            ram_fifo: spec.ram.fifo.clone(),
            font: spec.decode.as_ref().map(|d| d.font).unwrap_or_default(),
            digits: spec.decode.as_ref().and_then(|d| d.digits),
            meta,
            bytes: spec.bytes,
        })
    }

    /// The RAM as the artifact's bytes, in declaration order.
    ///
    /// Each var contributes its LOW BYTE. A var is an `i64` because the rule
    /// language is integer arithmetic; a display RAM cell is a byte, and the
    /// part's own rules are what keep it in range — `as u8` here is the same
    /// truncation the wire does.
    fn ram(&self, machine: &RuleMachine) -> Vec<u8> {
        if let Some(fifo) = &self.ram_fifo {
            let len = machine.fifo_len(fifo);
            return (0..len)
                .map(|i| machine.fifo_peek(fifo, i as u8).unwrap_or(0) as u8)
                .collect();
        }
        self.ram_vars
            .iter()
            .map(|name| machine.var(name) as u8)
            .collect()
    }

    /// Render the artifact for a device addressed by `id`.
    pub fn render(
        &self,
        machine: &RuleMachine,
        ctx: &dyn RuleCtx,
        id: &str,
        opts: &InspectOpts,
    ) -> Artifact {
        let ram = self.ram(machine);
        let mut meta = serde_json::Map::new();
        if let Some(format) = &self.format {
            meta.insert(
                "format".to_string(),
                serde_json::Value::String(format.clone()),
            );
        }
        // The cheap content hash a poller diffs instead of re-pulling bytes. It
        // is over the RAM the artifact PUBLISHES, so a part that keeps more RAM
        // than it shows (the TM1637 has six GRIDs and a four-digit module) does
        // not report a change nobody can see.
        meta.insert(
            "generation".to_string(),
            serde_json::Value::from(artifact_generation(&ram)),
        );
        if self.font == ArtifactFont::SevenSegment {
            let digits = self.digits.unwrap_or(ram.len()).min(ram.len());
            let text: String = ram[..digits]
                .iter()
                .map(|b| seven_seg_font::decode(*b))
                .collect();
            meta.insert("text".to_string(), serde_json::Value::String(text));
        }
        for field in &self.meta {
            let raw = match (&field.value, &field.source) {
                (Some(expr), _) => machine.eval_expr(expr, ctx),
                (None, Some(source)) => derived(source, &ram),
                // `validate` refuses a field with neither, at load.
                (None, None) => 0,
            };
            let value = match field.ty {
                ArtifactMetaType::Bool => serde_json::Value::Bool(raw != 0),
                ArtifactMetaType::Int => serde_json::Value::from(raw),
            };
            meta.insert(field.key.clone(), value);
        }
        Artifact {
            kind: self.kind.clone(),
            id: match &self.id_suffix {
                Some(suffix) => format!("{id}.{suffix}"),
                None => id.to_string(),
            },
            meta: serde_json::Value::Object(meta),
            bytes: if self.bytes {
                artifact_bytes(&ram, opts)
            } else {
                None
            },
        }
    }
}

/// The engine-derived `meta` quantities. See the module note for why these are
/// not expressions.
fn derived(source: &str, ram: &[u8]) -> i64 {
    match source {
        "lit_bits" => ram.iter().map(|b| i64::from(b.count_ones())).sum(),
        "ink_bytes" => ram.iter().filter(|&&b| b != 0).count() as i64,
        "bytes" => ram.len() as i64,
        // `validate` refuses any other spelling at load.
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::DeviceDescriptor;
    use std::collections::BTreeMap;

    /// A [`RuleCtx`] with no register file, the same one a pins-only part uses.
    struct NoRegs {
        slots: BTreeMap<String, f64>,
        scale: BTreeMap<String, f64>,
    }

    impl NoRegs {
        fn new() -> Self {
            Self {
                slots: BTreeMap::new(),
                scale: BTreeMap::new(),
            }
        }
    }

    impl RuleCtx for NoRegs {
        fn reg(&self, _: &str) -> Option<u32> {
            None
        }
        fn reported(&self, _: &str) -> Option<i64> {
            None
        }
        fn set_reg(&mut self, _: &str, _: u32) {}
        fn field_bits(&self, _: &str, _: &str) -> Option<(u8, u32)> {
            None
        }
        fn input(&self, key: &str) -> i64 {
            let _ = &self.scale;
            self.slots.get(key).copied().unwrap_or(0.0) as i64
        }
        fn set_input(&mut self, _: &str, _: i64) {}
    }

    const FIXTURE: &str = r#"
type: t
behavior:
  primitive: gpio_device
  pins: { A: a_pin }
  vars: { d0: 0, d1: 0, on: 0 }
  rules:
    - on: { pin: A, edge: rising }
      do: [ { var: { name: d0, value: 63 } } ]
  artifact:
    kind: text_display
    format: tm1637_grid
    ram: { vars: [d0, d1] }
    decode: { font: seven_segment, digits: 2 }
    meta:
      - { key: lit_segments, source: lit_bits }
      - { key: display_on, value: "var(on)", type: bool }
"#;

    fn compiled() -> (CompiledArtifact, RuleMachine) {
        let desc = DeviceDescriptor::from_yaml(FIXTURE).expect("parses");
        let art = CompiledArtifact::from_descriptor(&desc)
            .expect("compiles")
            .expect("declared");
        let machine = RuleMachine::from_behavior(&desc.behavior)
            .expect("compiles")
            .expect("has behaviour");
        (art, machine)
    }

    #[test]
    fn a_declared_artifact_renders_its_ram_through_the_font() {
        let (art, mut machine) = compiled();
        let mut ctx = NoRegs::new();
        machine.fire(
            &labwired_config::Event::Pin {
                name: "A".into(),
                edge: labwired_config::PinEdge::Rising,
            },
            0,
            &mut ctx,
        );
        let a = art.render(&machine, &ctx, "disp", &InspectOpts::default());
        assert_eq!(a.kind, "text_display");
        assert_eq!(a.id, "disp");
        assert_eq!(a.meta["format"], "tm1637_grid");
        // 0x3F is '0'; the second digit is still blank.
        assert_eq!(a.meta["text"], "0 ");
        // `lit_bits` is the popcount of the RAM — six segments for a '0'.
        assert_eq!(a.meta["lit_segments"], 6);
        // A `bool` field is a JSON boolean, not a 0/1 integer. The wasm
        // accessor reads `display_on` with `as_bool()`; an integer there is a
        // different artifact and reads as `None`.
        assert_eq!(a.meta["display_on"], serde_json::Value::Bool(false));
        // `bytes:` defaults to false — four or six RAM bytes are metadata-sized
        // and both hand-written segment models published `bytes: None`.
        assert!(a.bytes.is_none());
    }

    #[test]
    fn the_generation_moves_only_when_the_published_ram_moves() {
        let (art, mut machine) = compiled();
        let ctx = NoRegs::new();
        let before = art.render(&machine, &ctx, "d", &InspectOpts::default());
        // A var the artifact does NOT list must not move the generation, or a
        // poller re-pulls a panel that did not change.
        machine.set_observed_level("A", true);
        let same = art.render(&machine, &ctx, "d", &InspectOpts::default());
        assert_eq!(before.meta["generation"], same.meta["generation"]);
    }

    #[test]
    fn a_kind_no_surface_paints_is_a_load_error() {
        let yaml = FIXTURE.replace("kind: text_display", "kind: seven_segment_panel");
        let desc = DeviceDescriptor::from_yaml(&yaml).expect("parses");
        let err = CompiledArtifact::from_descriptor(&desc).unwrap_err();
        assert!(format!("{err:#}").contains("paintable kinds"), "{err:#}");
    }

    #[test]
    fn declaring_no_ram_is_a_load_error() {
        let yaml = FIXTURE.replace("ram: { vars: [d0, d1] }", "ram: {}");
        let desc = DeviceDescriptor::from_yaml(&yaml).expect("parses");
        let err = CompiledArtifact::from_descriptor(&desc).unwrap_err();
        assert!(format!("{err:#}").contains("exactly one of"), "{err:#}");
    }

    #[test]
    fn redeclaring_an_engine_stamped_key_is_a_load_error() {
        let yaml = FIXTURE.replace(
            "key: lit_segments, source: lit_bits",
            "key: generation, source: lit_bits",
        );
        let desc = DeviceDescriptor::from_yaml(&yaml).expect("parses");
        let err = CompiledArtifact::from_descriptor(&desc).unwrap_err();
        assert!(format!("{err:#}").contains("redeclares"), "{err:#}");
    }
}
