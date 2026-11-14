// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Every model a board actually registers must appear in its `models:` list.
//!
//! # Why
//!
//! `validation/boards/` (one file per board) records a `models:` path list per board, and
//! `scripts/generate_validation_status.py` hashes exactly those files into the
//! board's source digest. An omitted model can change without moving that
//! digest, concealing a difference from the source recorded with a capture.
//! Source drift is informational; complete source watches remain required.
//!
//! Not hypothetical, and not a one-off. The manifest already names one
//! instance in its own header ("the esp32c3 hole"). A second was found the
//! same way this gate would have: `nrf54l15` listed only `cpu/cortex_m.rs`,
//! `decoder/arm.rs` and its chip YAML — not `peripherals/nrf54l/{clock,uarte,
//! twim}.rs`, the models its own `note:` advertises. The walk-deletion work
//! rewrote all three and the digest did not move, while `atsamd21g18a`'s did,
//! because that board happens to list `sam/sercom_usart.rs`.
//!
//! # Why it asks the engine
//!
//! A chip YAML names its models two ways:
//!
//! ```text
//! path:  "../peripherals/<chip>/<block>.yaml"   a declarative register bank
//! type:  "nrf54l_uarte"                         a Rust model, via a factory
//! ```
//!
//! The first version of this gate mined `type:` -> source from the factory
//! match arms with a regex. It resolved 4 types out of 67 while reporting
//! itself complete: the arms come in too many shapes (qualified paths,
//! imported names, braced blocks, `a | b =>` alternations, one factory per
//! family). A gate that silently drops what it cannot parse is green exactly
//! where it is blind, which is worse than no gate.
//!
//! So this builds each board's bus through the SAME `SystemBus::from_config`
//! the product uses, and asks every registered peripheral what it is via
//! [`Peripheral::impl_type_path`]. There is no second implementation of the
//! dispatch chain to drift out of step with the first.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // crates/core -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

/// `labwired_core::peripherals::nrf54l::uarte::Nrf54lUarte` ->
/// `crates/core/src/peripherals/nrf54l/uarte.rs`.
///
/// Returns `None` for a type that lives outside `peripherals::`, or whose
/// module has no file of its own — those are reported, never skipped.
fn type_path_to_source(type_path: &str, root: &Path) -> Option<String> {
    // Drop any generic parameters: `Uart<Foo>` -> `Uart`.
    let bare = type_path.split('<').next().unwrap_or(type_path);
    let rest = bare.strip_prefix("labwired_core::")?;
    let mut segs: Vec<&str> = rest.split("::").collect();
    segs.pop()?; // the type name itself
    if segs.is_empty() {
        return None;
    }
    let base = root.join("crates/core/src").join(segs.join("/"));
    for cand in [base.with_extension("rs"), base.join("mod.rs")] {
        if cand.exists() {
            return Some(
                cand.strip_prefix(root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    None
}

/// Drop all but the LAST occurrence of a repeated board-level key.
///
/// the board files can carry duplicate `drift_ack:` keys — ten boards
/// do, up to sixteen times on `stm32h563`, `nucleo-l476rg` and `stm32f103`.
/// A YAML mapping is supposed to have unique keys; PyYAML silently keeps the
/// last, `serde_yaml` refuses the document outright.
///
/// This gate deliberately reproduces PyYAML's behaviour rather than rejecting,
/// because the question it answers is about the manifest AS THE GENERATOR SEES
/// IT. Reading it differently from `generate_validation_status.py` would make
/// this a check on a document that nothing else consumes.
///
/// The duplicates are a latent hazard, not a live bug: measured today, every
/// board's last-in-file ack is also its newest, so last-wins picks the right
/// one. It stops being true the moment an ack is appended or inserted out of
/// date order — the board then silently takes an older ack and can lapse early
/// or over-cover. Worth its own gate; out of scope for this one.
fn last_wins_duplicate_keys(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let board_key = |l: &str| -> Option<String> {
        let rest = l.strip_prefix("    ")?;
        if rest.starts_with(' ') || rest.starts_with('-') || rest.starts_with('#') {
            return None;
        }
        let k = rest.split(':').next()?;
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            return None;
        }
        Some(k.to_string())
    };

    // Board blocks start at `  - id:`; within each, keep only the final
    // occurrence of any repeated key.
    let mut drop = vec![false; lines.len()];
    let mut block_start = 0usize;
    let mut seen_last: BTreeMap<String, usize> = BTreeMap::new();
    let flush = |seen: &mut BTreeMap<String, usize>, drop: &mut Vec<bool>, _s: usize| {
        seen.clear();
        let _ = drop;
    };
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("  - id:") {
            flush(&mut seen_last, &mut drop, block_start);
            block_start = i;
        }
        if let Some(k) = board_key(l) {
            if let Some(prev) = seen_last.insert(k, i) {
                drop[prev] = true;
            }
        }
    }
    lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop[*i])
        .map(|(_, l)| *l)
        .collect::<Vec<_>>()
        .join("\n")
}

struct Board {
    id: String,
    chip: String,
    models: BTreeSet<String>,
}

/// `validation/boards/<id>.yaml` is one board's mapping each. Rebuild the
/// `boards:` list the old single manifest had (entry lines indented under a
/// `  - ` marker) so everything below sees the same text as before. Files load
/// in file-name order; `_index.yaml` is the header, not a board.
fn manifest_text(root: &Path) -> String {
    let dir = root.join("validation/boards");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| {
            p.extension().and_then(|s| s.to_str()) == Some("yaml")
                && p.file_name().and_then(|s| s.to_str()) != Some("_index.yaml")
        })
        .collect();
    paths.sort();
    let mut out = String::from("boards:\n");
    for p in paths {
        let text =
            std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
        let mut lines = text.lines().skip_while(|l| !l.starts_with("id:"));
        let Some(first) = lines.next() else {
            panic!("{}: no `id:` line", p.display());
        };
        out.push_str(&format!("  - {first}\n"));
        for l in lines {
            if l.is_empty() {
                out.push('\n');
            } else {
                out.push_str(&format!("    {l}\n"));
            }
        }
    }
    out
}

fn boards(root: &Path) -> Vec<Board> {
    let text = manifest_text(root);
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&last_wins_duplicate_keys(&text)).expect("parse manifest");
    let list = doc
        .get("boards")
        .and_then(|b| b.as_sequence())
        .expect("manifest has a `boards:` sequence");
    list.iter()
        .filter_map(|b| {
            let id = b.get("id")?.as_str()?.to_string();
            let chip = b.get("chip")?.as_str()?.to_string();
            let models: BTreeSet<String> = b
                .get("models")?
                .as_sequence()?
                .iter()
                .filter_map(|m| m.as_str().map(str::to_string))
                .collect();
            // A board recording no list at all is a different question, and
            // one this gate deliberately does not answer — see the module doc.
            if models.is_empty() {
                return None;
            }
            Some(Board { id, chip, models })
        })
        .collect()
}

/// chip yaml path -> a system yaml that wires it.
fn systems_by_chip(root: &Path) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    let dir = root.join("configs/systems");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    // SORTED. `read_dir` yields filesystem order, and several system YAMLs can
    // name the same chip — so an unsorted walk picks a different one per run,
    // the board registers a different peripheral set, and the gate reports a
    // different set of holes each time. That is how this gate first "found"
    // esp32c3 on one run and not the next.
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        let Some(rel) = doc.get("chip").and_then(|c| c.as_str()) else {
            continue;
        };
        let Ok(abs) = p.parent().unwrap().join(rel).canonicalize() else {
            continue;
        };
        let Ok(relp) = abs.strip_prefix(root) else {
            continue;
        };
        out.entry(relp.to_string_lossy().replace('\\', "/"))
            .or_insert(p.clone());
    }
    out
}

/// Boards whose `models:` list is known to be short, with the reason. This is
/// a RATCHET: an entry that stops being needed fails the gate, so the list can
/// only shrink. Remediating one means adding the paths to `models:` AND
/// re-acking that board's drift, which is a judgement about its capture that
/// belongs to whoever owns it — not something to do in bulk from here.
const ALLOWLIST: &[(&str, &str)] = &[
    (
        "nrf52840",
        "pre-existing: 33 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
    (
        "nucleo-l073rz",
        "pre-existing: 13 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
    (
        "nucleo-l476rg",
        "pre-existing: 18 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
    (
        "seeed-xiao-nrf52840-sense",
        "pre-existing: 33 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
    (
        "stm32f103",
        "pre-existing: 6 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
    (
        "stm32f407",
        "pre-existing: 11 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
    (
        "stm32h563",
        "pre-existing: 4 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
    (
        "esp32c3",
        "pre-existing: 9 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
    (
        "esp32s3",
        "pre-existing: 9 registered model(s) unlisted; wants the paths added AND a drift re-ack",
    ),
];

#[test]
fn every_registered_model_is_in_its_board_models_list() {
    let root = repo_root();
    let systems = systems_by_chip(&root);
    let allow: BTreeMap<&str, &str> = ALLOWLIST.iter().copied().collect();

    let mut holes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unmapped: BTreeSet<String> = BTreeSet::new();
    let mut checked = 0usize;
    let mut clean_but_allowlisted: Vec<String> = Vec::new();

    for board in boards(&root) {
        let Some(system_path) = systems.get(&board.chip) else {
            // Every board in the manifest should be buildable from some system
            // yaml. One that is not cannot be checked, and saying so is the
            // honest outcome — but it is not this gate's failure to report.
            continue;
        };
        let Ok(manifest) = SystemManifest::from_file(system_path) else {
            continue;
        };
        let Ok(chip) = ChipDescriptor::from_file(root.join(&board.chip)) else {
            continue;
        };
        let Ok(bus) = SystemBus::from_config(&chip, &manifest) else {
            continue;
        };
        checked += 1;

        let mut wanted: BTreeSet<String> = BTreeSet::new();
        for entry in &bus.peripherals {
            let tp = entry.dev.impl_type_path();
            match type_path_to_source(tp, &root) {
                Some(src) => {
                    wanted.insert(src);
                }
                None => {
                    unmapped.insert(tp.to_string());
                }
            }
        }

        let missing: Vec<String> = wanted
            .into_iter()
            .filter(|w| !board.models.contains(w))
            .collect();

        match (missing.is_empty(), allow.contains_key(board.id.as_str())) {
            (false, false) => {
                holes.insert(board.id.clone(), missing);
            }
            (true, true) => clean_but_allowlisted.push(board.id.clone()),
            _ => {}
        }
    }

    assert!(
        checked >= 20,
        "only {checked} boards were actually built — this gate is close to \
         vacuous. A build failure that silently `continue`s turns every board \
         it hits into a pass."
    );

    assert!(
        unmapped.is_empty(),
        "registered peripheral types this gate could not map to a source file:\n  {}\n\n\
         Not skipped, deliberately: a gate that drops what it cannot resolve is \
         green exactly where it is blind. Either the type lives outside \
         `labwired_core::peripherals::`, or its module has no file of its own.",
        unmapped.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );

    assert!(
        clean_but_allowlisted.is_empty(),
        "these boards are on ALLOWLIST but no longer have a hole — delete their \
         entries so the list keeps only shrinking: {clean_but_allowlisted:?}"
    );

    assert!(
        holes.is_empty(),
        "models registered by a board but absent from its `models:` list in \
         validation/boards/<id>.yaml:\n{}\n\n\
         These files are invisible to that board's source digest, so their \
         changes cannot be compared with captured source. Add them to `models:` \
         or record the board in ALLOWLIST with a reason. Source-review ACKs \
         are not required.",
        holes
            .iter()
            .map(|(b, ms)| format!("  {b} ({}):\n      {}", ms.len(), ms.join("\n      ")))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
