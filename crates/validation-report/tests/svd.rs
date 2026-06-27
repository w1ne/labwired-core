use std::path::{Path, PathBuf};
use validation_report::{
    report_from_tier1_matrix, svd_checks, tier1_checks, CheckStatus, ModelValidationReport,
};

fn svd_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/svd-descriptors")
}

#[test]
fn svd_checks_count_registers_and_skip_non_yaml() {
    let checks = svd_checks(&svd_dir()).unwrap();
    // uart0.yaml + empty0.yaml are read; notes.txt is skipped.
    assert_eq!(checks.len(), 2);

    let uart = checks.iter().find(|c| c.peripheral == "uart0").unwrap();
    assert_eq!(uart.status, CheckStatus::Pass);
    assert_eq!(
        uart.authority,
        "CMSIS-SVD register map (vendor register layout)"
    );
    assert_eq!(uart.detail.as_deref(), Some("2 registers"));
    assert_eq!(uart.evidence.as_deref(), Some("uart0.yaml"));

    // a descriptor with zero registers is unrecorded, never a silent pass
    let empty = checks.iter().find(|c| c.peripheral == "empty0").unwrap();
    assert_eq!(empty.status, CheckStatus::Unrecorded);
}

#[test]
fn multiple_authorities_merge_per_peripheral_and_count_once() {
    // tier-1 marks uart0 unrecorded; SVD validates its register layout (pass).
    // The merged peripheral status is pass, counted ONCE, with both authorities.
    let matrix = r#"{ "c3": { "uart0": { "status": "unrecorded" } } }"#;
    let mut checks = tier1_checks(matrix, "c3").unwrap();
    checks.extend(svd_checks(&svd_dir()).unwrap());
    let r = ModelValidationReport::from_checks("c3", checks);

    assert_eq!(r.summary.authorities, 2);
    // distinct peripherals: uart0 (pass via merge) + empty0 (unrecorded)
    assert_eq!(r.summary.pass, 1);
    assert_eq!(r.summary.unrecorded, 1);
    // uart0 carries TWO rows (one per authority) in the auditable table
    let uart_rows = r
        .peripherals
        .iter()
        .filter(|p| p.peripheral == "uart0")
        .count();
    assert_eq!(uart_rows, 2);
}

#[test]
fn tier1_only_report_is_unchanged_shape() {
    let matrix = r#"{ "esp32c3": { "uart": { "status": "pass" }, "i2c": { "status": "na" } } }"#;
    let r = report_from_tier1_matrix(matrix, "esp32c3").unwrap();
    assert_eq!(r.summary.authorities, 1);
    assert_eq!(r.summary.pass, 1);
    assert_eq!(r.summary.not_applicable, 1);
}
