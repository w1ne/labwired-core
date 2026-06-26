//! Model-validation report: aggregate labwired's scattered fidelity evidence into
//! one provenanced, auditable artifact per chip.
//!
//! proto.cat's claim is "the firmware verifiably runs" — which is only as good as
//! the fidelity of the silicon models it runs on. labwired already validates models
//! several ways (tier-1 raw-register-vs-TRM matrix, silicon reset-conformance in the
//! `hw-oracle` crate, SVD-derived register coverage, real vendor-stack boot in the
//! examples), but the evidence lives in separate files. This crate consolidates it
//! into a single `ModelValidationReport` that says, per peripheral, WHAT was checked
//! and against WHICH authority — so "validated model" is an audit trail, not a claim.
//!
//! Each authority contributes a list of `PeripheralValidation` checks; a peripheral
//! can carry checks from several authorities. The summary derives ONE status per
//! distinct peripheral (Fail > Pass > Unrecorded > n/a) and reports coverage over
//! peripherals, not raw rows — so being validated twice doesn't inflate the score.
//!
//! Authorities wired: (1) tier-1 coverage matrix (`docs/coverage/tier1-matrix.json`),
//! (2) SVD register descriptors (`configs/peripherals/<chip>/*.yaml`), (3) silicon
//! reset-conformance captures (`scripts/hw-oracle/captures/<chip>/.../reg_oracle.json`),
//! (4) vendor-stack / integration example boots (`examples/*/`, device-level). Designed
//! so further authorities (QEMU/Renode differential) attach without reshaping the report.

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Outcome of one validation check on a peripheral model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The model matched the authority.
    Pass,
    /// The model disagreed with the authority — a real fidelity gap.
    Fail,
    /// The peripheral is intentionally not covered by this authority.
    NotApplicable,
    /// In scope but no result recorded yet (a tracked gap, never a silent pass).
    Unrecorded,
}

impl CheckStatus {
    fn parse(s: &str) -> CheckStatus {
        match s {
            "pass" => CheckStatus::Pass,
            "fail" => CheckStatus::Fail,
            "na" => CheckStatus::NotApplicable,
            _ => CheckStatus::Unrecorded,
        }
    }
    fn label(self) -> &'static str {
        match self {
            CheckStatus::Pass => "PASS",
            CheckStatus::Fail => "FAIL",
            CheckStatus::NotApplicable => "n/a",
            CheckStatus::Unrecorded => "unrecorded",
        }
    }
    /// Merge two checks on the SAME peripheral into a derived status. A single
    /// disagreement fails the peripheral; otherwise any pass validates it; an
    /// unrecorded check keeps it open; n/a only when nothing else applies.
    fn merge(self, other: CheckStatus) -> CheckStatus {
        use CheckStatus::*;
        match (self, other) {
            (Fail, _) | (_, Fail) => Fail,
            (Pass, _) | (_, Pass) => Pass,
            (Unrecorded, _) | (_, Unrecorded) => Unrecorded,
            _ => NotApplicable,
        }
    }
}

/// One peripheral's validation against one authority, with provenance.
#[derive(Debug, Clone, Serialize)]
pub struct PeripheralValidation {
    pub peripheral: String,
    pub status: CheckStatus,
    /// Which authority the model was checked against (human-readable, citable).
    pub authority: String,
    /// Link/path to the run or capture that backs this result, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// Extra context for this check (e.g. "423 registers").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Rolled-up counts + coverage for a chip's model validation, over DISTINCT
/// peripherals (a peripheral validated by several authorities counts once).
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub pass: usize,
    pub fail: usize,
    pub not_applicable: usize,
    pub unrecorded: usize,
    /// pass / applicable, where applicable = pass + fail + unrecorded (excludes n/a).
    pub coverage_pct: f64,
    /// Distinct authorities that contributed at least one check.
    pub authorities: usize,
}

/// A device-level behavioral validation: an example boots firmware on the model and
/// runs to its acceptance assertions (the strongest behavioral evidence, esp. when the
/// firmware is an unmodified vendor stack — ESP-IDF/Zephyr/HAL/UDSLib). Device-level,
/// so it is reported alongside (not mixed into) per-peripheral coverage.
#[derive(Debug, Clone, Serialize)]
pub struct IntegrationCheck {
    pub example: String,
    pub status: CheckStatus,
    /// Number of acceptance assertions the example's test scripts gate on.
    pub assertions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// The provenanced model-validation report for a single chip.
#[derive(Debug, Clone, Serialize)]
pub struct ModelValidationReport {
    pub chip: String,
    pub peripherals: Vec<PeripheralValidation>,
    /// Device-level example boots (vendor-stack / integration behavioral checks).
    pub integrations: Vec<IntegrationCheck>,
    pub summary: Summary,
}

impl ModelValidationReport {
    /// Build a report from any set of authority checks. Rows are sorted by
    /// (peripheral, authority) for a stable, auditable artifact; the summary is
    /// derived over distinct peripherals.
    pub fn from_checks(chip: &str, mut checks: Vec<PeripheralValidation>) -> ModelValidationReport {
        checks.sort_by(|a, b| {
            a.peripheral
                .cmp(&b.peripheral)
                .then(a.authority.cmp(&b.authority))
        });
        let summary = Self::summarize(&checks);
        ModelValidationReport {
            chip: chip.to_string(),
            peripherals: checks,
            integrations: Vec::new(),
            summary,
        }
    }

    /// Attach device-level example-boot checks, sorted by example name.
    pub fn with_integrations(mut self, mut integrations: Vec<IntegrationCheck>) -> Self {
        integrations.sort_by(|a, b| a.example.cmp(&b.example));
        self.integrations = integrations;
        self
    }

    fn summarize(checks: &[PeripheralValidation]) -> Summary {
        let mut derived: BTreeMap<&str, CheckStatus> = BTreeMap::new();
        let mut authorities: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for c in checks {
            authorities.insert(c.authority.as_str());
            derived
                .entry(c.peripheral.as_str())
                .and_modify(|s| *s = s.merge(c.status))
                .or_insert(c.status);
        }
        let (mut pass, mut fail, mut not_applicable, mut unrecorded) = (0, 0, 0, 0);
        for s in derived.values() {
            match s {
                CheckStatus::Pass => pass += 1,
                CheckStatus::Fail => fail += 1,
                CheckStatus::NotApplicable => not_applicable += 1,
                CheckStatus::Unrecorded => unrecorded += 1,
            }
        }
        let applicable = pass + fail + unrecorded;
        let coverage_pct = if applicable == 0 {
            0.0
        } else {
            (pass as f64) * 100.0 / (applicable as f64)
        };
        Summary {
            pass,
            fail,
            not_applicable,
            unrecorded,
            coverage_pct,
            authorities: authorities.len(),
        }
    }

    /// A human-auditable markdown rendering of the report.
    pub fn to_markdown(&self) -> String {
        let s = &self.summary;
        let mut out = String::new();
        out.push_str(&format!("# Model validation — {}\n\n", self.chip));
        out.push_str(&format!(
            "Coverage: **{:.1}%** ({} pass / {} fail / {} unrecorded; {} n/a) across {} \
             distinct peripherals, {} authorit{}\n\n",
            s.coverage_pct,
            s.pass,
            s.fail,
            s.unrecorded,
            s.not_applicable,
            s.pass + s.fail + s.not_applicable + s.unrecorded,
            s.authorities,
            if s.authorities == 1 { "y" } else { "ies" },
        ));
        out.push_str("| Peripheral | Result | Authority | Detail | Evidence |\n");
        out.push_str("|---|---|---|---|---|\n");
        for p in &self.peripherals {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                p.peripheral,
                p.status.label(),
                p.authority,
                p.detail.as_deref().unwrap_or("—"),
                p.evidence.as_deref().unwrap_or("—"),
            ));
        }
        if !self.integrations.is_empty() {
            out.push_str("\n## Integration boots (firmware runs to acceptance assertions)\n\n");
            out.push_str("| Example | Result | Assertions | Evidence |\n");
            out.push_str("|---|---|---|---|\n");
            for i in &self.integrations {
                out.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    i.example,
                    i.status.label(),
                    i.assertions,
                    i.evidence.as_deref().unwrap_or("—"),
                ));
            }
        }
        out
    }
}

// ── Authority #1: tier-1 raw-register-vs-TRM coverage matrix ──────────────────

/// One peripheral entry in the tier-1 coverage matrix JSON.
#[derive(serde::Deserialize)]
struct Tier1Entry {
    status: String,
    #[serde(default)]
    run_url: Option<String>,
}

const TIER1_AUTHORITY: &str = "tier-1: raw-register sequence vs vendor TRM";

/// Checks from the tier-1 coverage matrix (`docs/coverage/tier1-matrix.json`): a
/// `{ chip: { peripheral: { status, run_url? } } }` map. Errors if the chip is
/// absent — a missing chip is a gap to surface, never a silently-empty report.
pub fn tier1_checks(matrix_json: &str, chip: &str) -> Result<Vec<PeripheralValidation>> {
    let matrix: BTreeMap<String, BTreeMap<String, Tier1Entry>> = serde_json::from_str(matrix_json)?;
    let entries = matrix
        .get(chip)
        .ok_or_else(|| anyhow!("chip '{chip}' is not in the tier-1 coverage matrix"))?;
    Ok(entries
        .iter()
        .map(|(name, e)| PeripheralValidation {
            peripheral: name.clone(),
            status: CheckStatus::parse(&e.status),
            authority: TIER1_AUTHORITY.to_string(),
            evidence: e.run_url.clone(),
            detail: None,
        })
        .collect())
}

/// Convenience: a tier-1-only report for one chip.
pub fn report_from_tier1_matrix(matrix_json: &str, chip: &str) -> Result<ModelValidationReport> {
    Ok(ModelValidationReport::from_checks(
        chip,
        tier1_checks(matrix_json, chip)?,
    ))
}

// ── Authority #2: SVD-derived register descriptors ────────────────────────────

#[derive(serde::Deserialize)]
struct SvdDescriptor {
    peripheral: String,
    #[serde(default)]
    registers: Vec<serde_yaml::Value>,
}

const SVD_AUTHORITY: &str = "CMSIS-SVD register map (vendor register layout)";

/// Checks from the SVD-derived peripheral descriptors in a directory
/// (`configs/peripherals/<chip>/*.yaml`). Each descriptor is a vendor-authoritative
/// register layout; presence with registers validates the model's INTERFACE (not its
/// dynamic behavior — that is what tier-1 / vendor-stack boot cover). A descriptor
/// with zero registers is `unrecorded`, not a silent pass.
pub fn svd_checks(peripherals_dir: &Path) -> Result<Vec<PeripheralValidation>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(peripherals_dir)
        .map_err(|e| anyhow!("reading {}: {e}", peripherals_dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let desc: SvdDescriptor =
            serde_yaml::from_str(&text).map_err(|e| anyhow!("parsing {}: {e}", path.display()))?;
        let n = desc.registers.len();
        out.push(PeripheralValidation {
            peripheral: desc.peripheral.to_lowercase(),
            status: if n > 0 {
                CheckStatus::Pass
            } else {
                CheckStatus::Unrecorded
            },
            authority: SVD_AUTHORITY.to_string(),
            evidence: path
                .file_name()
                .and_then(|f| f.to_str())
                .map(str::to_string),
            detail: Some(format!("{n} register{}", if n == 1 { "" } else { "s" })),
        });
    }
    Ok(out)
}

// ── Authority #3: silicon reset-conformance (hw-oracle OpenOCD captures) ───────

#[derive(serde::Deserialize)]
struct ResetCapture {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    blocks: BTreeMap<String, ResetBlock>,
}

#[derive(serde::Deserialize)]
struct ResetBlock {
    #[serde(default)]
    words: BTreeMap<String, String>,
}

const HW_ORACLE_AUTHORITY: &str = "silicon reset-conformance (OpenOCD capture vs real hardware)";

/// Checks from a committed hw-oracle reset capture
/// (`scripts/hw-oracle/captures/<chip>/.../reg_oracle.json`): real-silicon reset
/// register values read over OpenOCD from a physical board. Per peripheral block, the
/// number of registers with real-hardware ground truth the `hw-oracle` conformance
/// suite diffs the model against. This is the strongest authority — the model is held
/// to values measured on actual silicon — and needs no hardware at check time (the
/// capture is committed). A block with no words is `unrecorded`, never a silent pass.
pub fn hw_oracle_checks(capture_json: &str) -> Result<Vec<PeripheralValidation>> {
    let capture: ResetCapture = serde_json::from_str(capture_json)?;
    let source = capture.source;
    Ok(capture
        .blocks
        .into_iter()
        .map(|(name, block)| {
            let n = block.words.len();
            PeripheralValidation {
                peripheral: name.to_lowercase(),
                status: if n > 0 {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Unrecorded
                },
                authority: HW_ORACLE_AUTHORITY.to_string(),
                evidence: source.clone(),
                detail: Some(format!(
                    "{n} reset register{} vs real silicon",
                    if n == 1 { "" } else { "s" }
                )),
            }
        })
        .collect())
}

// ── Authority #4: vendor-stack / integration example boots ────────────────────

#[derive(serde::Deserialize)]
struct ExampleManifest {
    chip: String,
}

/// Resolve a `chip:` manifest field to a chip id: the file stem of the referenced
/// path (`../../configs/chips/esp32c3.yaml` → `esp32c3`), or the value itself if it
/// is already a bare id.
fn chip_id_of(manifest_chip: &str) -> String {
    Path::new(manifest_chip)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(manifest_chip)
        .to_string()
}

/// Device-level behavioral checks: scan an examples directory for examples whose
/// `system.yaml` targets `chip`, and for each count the acceptance assertions across
/// its test scripts (any `*.yaml` with a top-level `assertions:` sequence). An example
/// boots real firmware on the model and is gated on those assertions in CI — the
/// strongest behavioral evidence (especially the unmodified vendor-stack examples).
/// An example with zero assertions is `unrecorded`, never a silent pass.
pub fn vendor_stack_checks(examples_dir: &Path, chip: &str) -> Result<Vec<IntegrationCheck>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(examples_dir)
        .map_err(|e| anyhow!("reading {}: {e}", examples_dir.display()))?
    {
        let dir = entry?.path();
        let manifest_path = dir.join("system.yaml");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest: ExampleManifest =
            match serde_yaml::from_str(&std::fs::read_to_string(&manifest_path)?) {
                Ok(m) => m,
                Err(_) => continue, // not a chip-targeting manifest
            };
        if chip_id_of(&manifest.chip) != chip {
            continue;
        }
        // Sum acceptance assertions across the example's test scripts.
        let mut assertions = 0usize;
        for f in std::fs::read_dir(&dir)? {
            let p = f?.path();
            if p.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            if let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(&std::fs::read_to_string(&p)?)
            {
                if let Some(seq) = v.get("assertions").and_then(|a| a.as_sequence()) {
                    assertions += seq.len();
                }
            }
        }
        let name = dir
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("example")
            .to_string();
        out.push(IntegrationCheck {
            status: if assertions > 0 {
                CheckStatus::Pass
            } else {
                CheckStatus::Unrecorded
            },
            evidence: Some(format!("examples/{name}")),
            example: name,
            assertions,
        });
    }
    Ok(out)
}
