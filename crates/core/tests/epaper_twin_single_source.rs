// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Manifest-fork guard: one physical module, one twin.
//!
//! `configs/systems/esp32-wroom-epaper.yaml` and
//! `examples/esp32-epaper-lab/system.yaml` describe the same board — ESP32-WROOM-32
//! with the 2.9" tri-color e-paper on VSPI, CS=GPIO5 DC=GPIO17 — and carried a
//! "kept in sync manually" comment. They drifted anyway: one said
//! `ssd1680_tricolor_290`, the other `uc8151d_tricolor_290`. Those two models share
//! nothing at the opcode level, so the *same* firmware paints against one and hangs
//! in `_waitWhileBusy` against the other. The drifted file is the one the playground
//! bundles for the flagship labwired-ereader demo.
//!
//! A comment cannot enforce this; this test can. The invariant:
//!
//!   **Two manifests that wire the same chip + bus + CS + DC must not name
//!   different members of the same part family.**
//!
//! "Part family" is derived from the registry, not from a hand-maintained list:
//! kit `device_type`s are spelled `<controller>_<form_factor>`, so
//! `ssd1680_tricolor_290` and `uc8151d_tricolor_290` share the family
//! `tricolor_290`. Today that is the only multi-member family, which is exactly
//! the pair that bit us. A new controller for an existing form factor joins its
//! family automatically.
//!
//! Scope covers committed YAML manifests *and* the manifests inlined as Rust raw
//! strings in tests / the wasm fallback — those inline copies are forks too, and
//! two of them had already drifted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One physical wiring: `(chip, connection, cs_pin, dc_pin, part_family)`. Two
/// manifests that agree on all five describe the same module.
type WiringKey = (String, String, String, String, String);

/// `device_type` → the manifests that name it for a given [`WiringKey`]. More
/// than one entry means the wiring has forked.
type ModelSources = BTreeMap<String, Vec<String>>;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Resolve a manifest `type:` through the same alias table the simulator uses,
/// so `gxepd2_290_c90c` and `ssd1680_tricolor_290` compare equal. Driver-class →
/// controller is owned by `peripherals::kit::registry` and nowhere else.
fn canonical_device_type(raw: &str) -> String {
    labwired_core::peripherals::kit::registry::lookup(raw)
        .map(|kit| kit.metadata().device_type.to_string())
        .unwrap_or_else(|| raw.to_string())
}

/// `ssd1680_tricolor_290` → `Some("tricolor_290")`. Kits whose `device_type` has
/// no `_` (e.g. `ili9341`) are single-member by construction and can't fork.
fn part_family(device_type: &str) -> Option<&str> {
    device_type.split_once('_').map(|(_, family)| family)
}

/// Families with more than one registered controller — the only ones where
/// "which model?" is a real question. Derived from the live registry.
fn contested_families() -> Vec<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for kit in labwired_core::peripherals::kit::registry::kits() {
        if let Some(f) = part_family(kit.metadata().device_type) {
            *counts.entry(f).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(f, _)| f.to_string())
        .collect()
}

/// One external-device declaration, reduced to the fields that identify the
/// physical wiring.
#[derive(Debug)]
struct Wiring {
    chip: String,
    connection: String,
    cs_pin: String,
    dc_pin: String,
    device_type: String,
    source: String,
}

fn str_field(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    map.get(serde_yaml::Value::String(key.to_string()))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Pull every `external_devices` entry out of one parsed manifest.
fn wirings_from_value(doc: &serde_yaml::Value, source: &str, out: &mut Vec<Wiring>) {
    let Some(map) = doc.as_mapping() else { return };
    // `chip:` is either a bare key (`esp32`) or a path (`../chips/esp32.yaml`).
    let chip = str_field(map, "chip")
        .map(|c| {
            Path::new(&c)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or(c)
        })
        .unwrap_or_default();
    let Some(devices) = map
        .get(serde_yaml::Value::String("external_devices".into()))
        .and_then(|v| v.as_sequence())
    else {
        return;
    };
    for dev in devices {
        let Some(dev_map) = dev.as_mapping() else {
            continue;
        };
        let Some(raw_type) = str_field(dev_map, "type") else {
            continue;
        };
        let cfg = dev_map
            .get(serde_yaml::Value::String("config".into()))
            .and_then(|v| v.as_mapping());
        let get_cfg = |k: &str| {
            cfg.and_then(|m| str_field(m, k))
                .unwrap_or_else(|| "-".to_string())
        };
        out.push(Wiring {
            chip: chip.clone(),
            connection: str_field(dev_map, "connection").unwrap_or_default(),
            cs_pin: get_cfg("cs_pin"),
            dc_pin: get_cfg("dc_pin"),
            device_type: canonical_device_type(&raw_type),
            source: source.to_string(),
        });
    }
}

/// Every Rust raw-string literal (`r#"..."#`) in `text`. The inline manifests are
/// all written this way.
fn raw_string_literals(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(rel) = text[i..].find("r#\"") {
        let start = i + rel + 3;
        match text[start..].find("\"#") {
            Some(end_rel) => {
                out.push(&text[start..start + end_rel]);
                i = start + end_rel + 2;
            }
            None => break,
        }
        if i >= bytes.len() {
            break;
        }
    }
    out
}

/// Walk `dir` collecting files whose extension matches, skipping build output.
fn walk(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == "node_modules" || name == ".git" || name == ".pio" {
            continue;
        }
        if path.is_dir() {
            walk(&path, exts, out);
        } else if path
            .extension()
            .map(|e| exts.contains(&e.to_string_lossy().as_ref()))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

/// Collect every external-device wiring declared anywhere in the repo.
fn all_wirings() -> Vec<Wiring> {
    let root = repo_root();
    let mut out = Vec::new();

    // 1. Committed YAML manifests.
    let mut yaml_files = Vec::new();
    for sub in ["configs/systems", "examples", "platformio"] {
        walk(&root.join(sub), &["yaml", "yml"], &mut yaml_files);
    }
    for path in yaml_files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains("external_devices") {
            continue;
        }
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        wirings_from_value(&doc, &rel, &mut out);
    }

    // 2. Manifests inlined as Rust raw strings — forks of the YAML above.
    let mut rs_files = Vec::new();
    walk(&root.join("crates"), &["rs"], &mut rs_files);
    for path in rs_files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains("external_devices") {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        for lit in raw_string_literals(&text) {
            if !lit.contains("external_devices") {
                continue;
            }
            let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(lit) else {
                continue;
            };
            wirings_from_value(&doc, &format!("{rel} (inline)"), &mut out);
        }
    }

    assert!(
        out.len() > 20,
        "collected only {} external-device declarations — the sweep is not finding \
         the manifests, so this guard would pass vacuously",
        out.len()
    );
    out
}

/// The guard. Same board + same pins ⇒ same twin.
#[test]
fn same_wiring_must_not_name_two_models_from_one_part_family() {
    let families = contested_families();
    assert!(
        !families.is_empty(),
        "no part family has two registered controllers — this guard has nothing to \
         check and must be re-derived before it is trusted"
    );

    let wirings = all_wirings();
    let mut groups: BTreeMap<WiringKey, ModelSources> = BTreeMap::new();

    for w in &wirings {
        let Some(family) = part_family(&w.device_type) else {
            continue;
        };
        if !families.iter().any(|f| f == family) {
            continue;
        }
        groups
            .entry((
                w.chip.clone(),
                w.connection.clone(),
                w.cs_pin.clone(),
                w.dc_pin.clone(),
                family.to_string(),
            ))
            .or_default()
            .entry(w.device_type.clone())
            .or_default()
            .push(w.source.clone());
    }

    assert!(
        !groups.is_empty(),
        "no manifest wires a contested part family ({families:?}) — guard is vacuous"
    );

    let mut failures = String::new();
    for ((chip, conn, cs, dc, family), models) in &groups {
        if models.len() > 1 {
            failures.push_str(&format!(
                "\n  {chip} / {conn} / CS={cs} DC={dc} — one board, {} different \
                 '{family}' twins:\n",
                models.len()
            ));
            for (model, sources) in models {
                for s in sources {
                    failures.push_str(&format!("      {model:<24} <- {s}\n"));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "manifest fork: the same physical wiring is described by more than one \
         panel/controller model. The same firmware cannot paint against both — one \
         of these is a blank screen in the product. Fix the manifest; do not relax \
         this test.\n{failures}"
    );
}

/// FIDELITY.md §E claims every live ESP32 e-paper path wires `dc_pin`, so the
/// command-vs-data INFER fallback in the panel models is unreachable there. That
/// is true today and this pins it: the real modules are 4-wire SPI with a genuine
/// DC line, and a manifest that omits it silently downgrades a modelled wire to a
/// protocol-state guess.
///
/// Deliberately scoped to the ESP32 families. The two STM32F103 e-paper manifests
/// (`examples/epaper-tricolor-lab`, `configs/systems/nucleo-f103rb-epaper.yaml`)
/// declare no `dc_pin` and DO still reach INFER; making `dc_pin` mandatory in
/// `PeripheralKit::attach` would break those shipping labs, so the claim is
/// enforced where it is actually made rather than widened here.
#[test]
fn esp32_epaper_manifests_must_wire_a_real_dc_pin() {
    let offenders: Vec<String> = all_wirings()
        .into_iter()
        .filter(|w| part_family(&w.device_type) == Some("tricolor_290"))
        .filter(|w| w.chip.starts_with("esp32"))
        .filter(|w| w.dc_pin == "-")
        .map(|w| format!("      {} ({}) <- {}", w.device_type, w.chip, w.source))
        .collect();

    assert!(
        offenders.is_empty(),
        "e-paper panel attached on an ESP32 path with no dc_pin — the model falls \
         back to inferring command-vs-data from protocol state (CHEAT(INFER)) \
         instead of reading the real GPIO. Wire the DC line in the manifest:\n{}",
        offenders.join("\n")
    );
}
