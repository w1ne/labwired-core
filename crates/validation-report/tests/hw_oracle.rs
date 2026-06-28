use validation_report::{hw_oracle_checks, tier1_checks, CheckStatus, ModelValidationReport};

const CAPTURE: &str = include_str!("fixtures/reg_oracle.json");

#[test]
fn counts_silicon_corroborated_registers_per_block() {
    let checks = hw_oracle_checks(CAPTURE).unwrap();
    assert_eq!(checks.len(), 3);

    let uart = checks.iter().find(|c| c.peripheral == "uart0").unwrap();
    assert_eq!(uart.status, CheckStatus::Pass);
    assert_eq!(
        uart.authority,
        "silicon reset-conformance (OpenOCD capture vs real hardware)"
    );
    assert_eq!(
        uart.detail.as_deref(),
        Some("2 reset registers vs real silicon")
    );
    assert!(uart.evidence.as_deref().unwrap().contains("openocd"));

    // singular grammar + a block with no captured words is unrecorded, never a pass
    let spi = checks.iter().find(|c| c.peripheral == "spi2").unwrap();
    assert_eq!(
        spi.detail.as_deref(),
        Some("1 reset register vs real silicon")
    );
    let empty = checks.iter().find(|c| c.peripheral == "empty").unwrap();
    assert_eq!(empty.status, CheckStatus::Unrecorded);
}

#[test]
fn three_authorities_merge_into_one_report() {
    let matrix = r#"{ "c3": { "uart0": { "status": "pass" } } }"#;
    let mut checks = tier1_checks(matrix, "c3").unwrap();
    checks.extend(hw_oracle_checks(CAPTURE).unwrap());
    let r = ModelValidationReport::from_checks("c3", checks);

    assert_eq!(r.summary.authorities, 2); // tier-1 + hw-oracle
                                          // uart0 validated by both authorities → counted once, two audit rows
    assert_eq!(
        r.peripherals
            .iter()
            .filter(|p| p.peripheral == "uart0")
            .count(),
        2
    );
    let uart_pass = r
        .peripherals
        .iter()
        .filter(|p| p.peripheral == "uart0" && p.status == CheckStatus::Pass)
        .count();
    assert_eq!(uart_pass, 2);
}
