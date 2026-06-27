use std::path::{Path, PathBuf};
use validation_report::{report_from_tier1_matrix, vendor_stack_checks, CheckStatus};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/examples")
}

#[test]
fn finds_examples_for_the_chip_and_counts_assertions() {
    let checks = vendor_stack_checks(&examples_dir(), "esp32c3").unwrap();
    // good-c3 (matches, has assertions) + noassert-c3 (matches, none); NOT other-f103
    let names: Vec<_> = checks.iter().map(|c| c.example.as_str()).collect();
    assert!(names.contains(&"good-c3"));
    assert!(names.contains(&"noassert-c3"));
    assert!(!names.contains(&"other-f103"));

    let good = checks.iter().find(|c| c.example == "good-c3").unwrap();
    assert_eq!(good.status, CheckStatus::Pass);
    assert_eq!(good.assertions, 3);
    assert_eq!(good.evidence.as_deref(), Some("examples/good-c3"));

    // an example with no acceptance assertions is unrecorded, never a silent pass
    let none = checks.iter().find(|c| c.example == "noassert-c3").unwrap();
    assert_eq!(none.status, CheckStatus::Unrecorded);
    assert_eq!(none.assertions, 0);
}

#[test]
fn integrations_render_separately_from_peripheral_coverage() {
    let matrix = r#"{ "esp32c3": { "uart": { "status": "pass" } } }"#;
    let report = report_from_tier1_matrix(matrix, "esp32c3")
        .unwrap()
        .with_integrations(vendor_stack_checks(&examples_dir(), "esp32c3").unwrap());

    // peripheral coverage is untouched by integration checks
    assert_eq!(report.summary.pass, 1);
    assert_eq!(report.peripherals.len(), 1);
    // integrations are a separate, sorted list
    assert_eq!(report.integrations.len(), 2);
    assert_eq!(report.integrations[0].example, "good-c3");

    let md = report.to_markdown();
    assert!(md.contains("## Integration boots"));
    assert!(md.contains("good-c3"));
}
