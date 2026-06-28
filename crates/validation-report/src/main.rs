//! `validation-report <tier1-matrix.json> <chip> [peripherals-dir] [--json]`
//!
//! Prints a chip's provenanced model-validation report (markdown by default, JSON
//! with --json). Aggregates the tier-1 matrix plus, when a peripherals descriptor
//! directory is given (or auto-found at `configs/peripherals/<chip>`), the SVD
//! register-layout authority.
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use validation_report::{
    hw_oracle_checks, svd_checks, tier1_checks, vendor_stack_checks, ModelValidationReport,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let positional: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with("--")).collect();
    let matrix_path = positional.first().ok_or_else(|| {
        anyhow!("usage: validation-report <tier1-matrix.json> <chip> [peripherals-dir] [--json]")
    })?;
    let chip = positional.get(1).ok_or_else(|| {
        anyhow!("usage: validation-report <tier1-matrix.json> <chip> [peripherals-dir] [--json]")
    })?;
    let as_json = args.iter().any(|a| a == "--json");

    let matrix_json = std::fs::read_to_string(matrix_path)?;
    let mut checks = tier1_checks(&matrix_json, chip)?;

    // SVD authority: explicit dir, else auto-find configs/peripherals/<chip>.
    let svd_dir = positional
        .get(2)
        .map(|p| PathBuf::from(p.as_str()))
        .unwrap_or_else(|| PathBuf::from(format!("configs/peripherals/{chip}")));
    if svd_dir.is_dir() {
        checks.extend(svd_checks(&svd_dir)?);
    }

    // hw-oracle authority: auto-find a committed silicon reset capture for the chip
    // at scripts/hw-oracle/captures/<chip>/.../reg_oracle.json.
    if let Some(capture) = find_reset_capture(chip) {
        checks.extend(hw_oracle_checks(&std::fs::read_to_string(&capture)?)?);
    }

    // vendor-stack / integration authority: example boots targeting this chip.
    let examples = PathBuf::from("examples");
    let integrations = if examples.is_dir() {
        vendor_stack_checks(&examples, chip)?
    } else {
        Vec::new()
    };

    let report = ModelValidationReport::from_checks(chip, checks).with_integrations(integrations);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.to_markdown());
    }
    Ok(())
}

/// Find a committed silicon reset capture for `chip` under
/// `scripts/hw-oracle/captures/<chip>/` (captures live in timestamped subdirs).
fn find_reset_capture(chip: &str) -> Option<PathBuf> {
    let root = PathBuf::from(format!("scripts/hw-oracle/captures/{chip}"));
    let direct = root.join("reg_oracle.json");
    if direct.is_file() {
        return Some(direct);
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("reg_oracle.json"))
        .filter(|p| p.is_file())
        .collect();
    found.sort(); // timestamped dir names sort chronologically; take the latest
    found.pop()
}
