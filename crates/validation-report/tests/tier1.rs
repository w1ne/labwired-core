use validation_report::{report_from_tier1_matrix, CheckStatus};

const MATRIX: &str = r#"{
  "esp32c3": {
    "uart":  { "status": "pass", "run_url": "https://ci/run/1" },
    "gpio":  { "status": "pass" },
    "i2c":   { "status": "na" },
    "spi":   { "status": "fail" },
    "timer": { "status": "unrecorded" }
  },
  "esp32": { "gpio": { "status": "pass" } }
}"#;

#[test]
fn builds_a_per_chip_report_with_provenance() {
    let r = report_from_tier1_matrix(MATRIX, "esp32c3").unwrap();
    assert_eq!(r.chip, "esp32c3");
    // peripherals are sorted by name for a stable, auditable report
    let names: Vec<_> = r
        .peripherals
        .iter()
        .map(|p| p.peripheral.as_str())
        .collect();
    assert_eq!(names, vec!["gpio", "i2c", "spi", "timer", "uart"]);

    let uart = r
        .peripherals
        .iter()
        .find(|p| p.peripheral == "uart")
        .unwrap();
    assert_eq!(uart.status, CheckStatus::Pass);
    assert_eq!(
        uart.authority,
        "tier-1: raw-register sequence vs vendor TRM"
    );
    assert_eq!(uart.evidence.as_deref(), Some("https://ci/run/1"));
}

#[test]
fn summary_counts_and_coverage_exclude_not_applicable() {
    let r = report_from_tier1_matrix(MATRIX, "esp32c3").unwrap();
    assert_eq!(r.summary.pass, 2); // uart, gpio
    assert_eq!(r.summary.fail, 1); // spi
    assert_eq!(r.summary.not_applicable, 1); // i2c
    assert_eq!(r.summary.unrecorded, 1); // timer
                                         // coverage = pass / applicable(pass+fail+unrecorded) = 2/4 = 50%
    assert!((r.summary.coverage_pct - 50.0).abs() < 1e-9);
}

#[test]
fn unknown_chip_is_an_error_not_a_silent_empty_report() {
    assert!(report_from_tier1_matrix(MATRIX, "stm32h563").is_err());
}

#[test]
fn renders_json_and_markdown() {
    let r = report_from_tier1_matrix(MATRIX, "esp32c3").unwrap();
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("\"chip\":\"esp32c3\""));
    assert!(json.contains("coverage_pct"));

    let md = r.to_markdown();
    assert!(md.contains("# Model validation — esp32c3"));
    assert!(md.contains("uart"));
    assert!(md.contains("PASS"));
    assert!(md.contains("50.0%"));
}
