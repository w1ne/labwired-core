use super::*;

fn script(schema: &str, block: &str) -> String {
    format!(
        r#"
schema_version: "{schema}"
inputs:
  firmware: "cow.elf"
  system: "sys.yaml"
limits:
  max_steps: 1000
{block}
"#
    )
}

#[test]
fn parses_text_and_raw_bytes_on_1_2() {
    let yaml = script(
        "1.2",
        r#"uart_injections:
  - uart: "uart1"
    bytes: "AB"
    trigger: !after_cycles { cycles: 500 }
  - uart: "uart2"
    bytes: [0x51, 0x00, 0xFF]
"#,
    );
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    s.validate().unwrap();
    assert_eq!(s.uart_injections.len(), 2);
    assert_eq!(s.uart_injections[0].uart, "uart1");
    assert_eq!(s.uart_injections[0].bytes.as_bytes(), b"AB".to_vec());
    assert!(matches!(
        s.uart_injections[0].trigger,
        FaultTrigger::AfterCycles { cycles: 500 }
    ));
    // Default trigger is at_start.
    assert!(matches!(
        s.uart_injections[1].trigger,
        FaultTrigger::AtStart
    ));
    assert_eq!(
        s.uart_injections[1].bytes.as_bytes(),
        vec![0x51, 0x00, 0xFF]
    );
}

#[test]
fn requires_schema_1_2() {
    for schema in ["1.0", "1.1"] {
        let yaml = script(
            schema,
            "uart_injections:\n  - uart: \"uart1\"\n    bytes: \"A\"\n",
        );
        let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
        let err = s.validate().unwrap_err().to_string();
        assert!(err.contains("require schema_version '1.2'"), "{err}");
    }
}

#[test]
fn empty_uart_id_rejected() {
    let yaml = script(
        "1.2",
        "uart_injections:\n  - uart: \"\"\n    bytes: \"A\"\n",
    );
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    assert!(s
        .validate()
        .unwrap_err()
        .to_string()
        .contains("'uart' cannot be empty"));
}

#[test]
fn empty_bytes_rejected() {
    let yaml = script(
        "1.2",
        "uart_injections:\n  - uart: \"uart1\"\n    bytes: \"\"\n",
    );
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    assert!(s
        .validate()
        .unwrap_err()
        .to_string()
        .contains("'bytes' cannot be empty"));
}

#[test]
fn register_triggers_rejected() {
    let yaml = script(
        "1.2",
        r#"uart_injections:
  - uart: "uart1"
    bytes: "A"
    trigger: !on_write { register: "FOO" }
"#,
    );
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    assert!(s
        .validate()
        .unwrap_err()
        .to_string()
        .contains("not yet supported for uart_injections"));
}

#[test]
fn malformed_bytes_rejected_at_parse() {
    // `bytes` must be either a string or an array of numbers; a mapping
    // matches neither untagged variant and fails to parse.
    let yaml = script(
        "1.2",
        "uart_injections:\n  - uart: \"uart1\"\n    bytes: { nope: true }\n",
    );
    assert!(serde_yaml::from_str::<TestScript>(&yaml).is_err());
}

#[test]
fn absent_field_parses_and_behaves_like_before() {
    // A script written before this field existed must still parse, with
    // `uart_injections` defaulting to empty (schema unaffected).
    let yaml = script("1.0", "assertions: []");
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    s.validate().unwrap();
    assert!(s.uart_injections.is_empty());
}

fn display_script(body: &str) -> TestScript {
    serde_yaml::from_str(&script("1.0", body)).expect("display_region must parse")
}

#[test]
fn display_region_parses_flat_with_defaults() {
    let s =
        display_script("assertions:\n  - display_region:\n      id: \"tft\"\n      min_ink: 0.5\n");
    s.validate().unwrap();
    let TestAssertion::DisplayRegion(a) = &s.assertions[0] else {
        panic!(
            "expected a display_region assertion, got {:?}",
            s.assertions[0]
        );
    };
    let d = &a.display_region;
    assert_eq!(d.id, "tft");
    assert_eq!((d.x, d.y), (0, 0), "origin defaults to the top-left");
    assert_eq!(
        (d.w, d.h),
        (None, None),
        "an absent size means the rest of the panel, decided against real geometry"
    );
    assert_eq!(d.max_ink, None);
}

/// The guard that stops this assertion from becoming decoration. `min_ink:
/// 0.0` with no ceiling admits every framebuffer, including one the firmware
/// never wrote — it would read as display coverage while proving nothing.
#[test]
fn display_region_without_a_bound_is_rejected_as_vacuous() {
    let s =
        display_script("assertions:\n  - display_region:\n      id: \"tft\"\n      min_ink: 0.0\n");
    let err = s.validate().unwrap_err().to_string();
    assert!(
        err.contains("accepts every possible framebuffer"),
        "unexpected error: {err}"
    );

    // The same region WITH a ceiling is a real claim ("this stayed clear").
    let ok = display_script(
            "assertions:\n  - display_region:\n      id: \"tft\"\n      min_ink: 0.0\n      max_ink: 0.0\n",
        );
    ok.validate().unwrap();
}

#[test]
fn display_region_rejects_impossible_bounds() {
    for (body, needle) in [
            (
                "assertions:\n  - display_region:\n      id: \"tft\"\n      min_ink: 1.5\n",
                "fraction in 0.0..=1.0",
            ),
            (
                "assertions:\n  - display_region:\n      id: \"tft\"\n      min_ink: 0.9\n      max_ink: 0.2\n",
                "is below min_ink",
            ),
            (
                "assertions:\n  - display_region:\n      id: \"\"\n      min_ink: 0.5\n",
                "id cannot be empty",
            ),
        ] {
            let err = display_script(body).validate().unwrap_err().to_string();
            assert!(err.contains(needle), "expected {needle:?}, got: {err}");
        }
}

/// `deny_unknown_fields` plus the untagged enum means a typo'd key does not
/// quietly become some other assertion variant.
#[test]
fn display_region_typo_does_not_parse_as_something_else() {
    let yaml = script(
            "1.0",
            "assertions:\n  - display_region:\n      id: \"tft\"\n      min_ink: 0.5\n      mn_ink: 0.9\n",
        );
    assert!(serde_yaml::from_str::<TestScript>(&yaml).is_err());
}

#[test]
fn rtt_contains_parses_as_its_own_variant() {
    let yaml = script("1.0", "assertions:\n  - rtt_contains: \"RTT hello\"");
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    assert!(matches!(
        s.assertions.as_slice(),
        [TestAssertion::RttContains(a)] if a.rtt_contains == "RTT hello"
    ));
}

/// `deny_unknown_fields` plus the untagged enum means a typo'd key does not
/// quietly become some other assertion variant.
#[test]
fn rtt_contains_typo_does_not_parse_as_something_else() {
    let yaml = script("1.0", "assertions:\n  - rtt_contians: \"RTT hello\"");
    let err = serde_yaml::from_str::<TestScript>(&yaml).unwrap_err();
    assert!(
        err.to_string().contains("did not match any variant"),
        "unexpected error: {err}"
    );
}

#[test]
fn empty_semihosting_contains_is_rejected_by_validate() {
    let yaml = script("1.0", "assertions:\n  - semihosting_contains: \"\"");
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    let err = s.validate().unwrap_err();
    assert!(
        err.to_string()
            .contains("semihosting_contains cannot be empty"),
        "unexpected error: {err}"
    );
}

#[test]
fn itm_contains_parses_as_its_own_variant() {
    let yaml = script("1.0", "assertions:\n  - itm_contains: \"ITM hello\"");
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    assert!(matches!(
        s.assertions.as_slice(),
        [TestAssertion::ItmContains(a)] if a.itm_contains == "ITM hello"
    ));
}

#[test]
fn itm_contains_typo_does_not_parse_as_something_else() {
    let yaml = script("1.0", "assertions:\n  - itm_contians: \"ITM hello\"");
    let err = serde_yaml::from_str::<TestScript>(&yaml).unwrap_err();
    assert!(
        err.to_string().contains("did not match any variant"),
        "unexpected error: {err}"
    );
}

#[test]
fn empty_itm_contains_is_rejected_by_validate() {
    let yaml = script("1.0", "assertions:\n  - itm_contains: \"\"");
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    let err = s.validate().unwrap_err();
    assert!(
        err.to_string().contains("itm_contains cannot be empty"),
        "unexpected error: {err}"
    );
}

#[test]
fn empty_rtt_contains_is_rejected_by_validate() {
    let yaml = script("1.0", "assertions:\n  - rtt_contains: \"\"");
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    let err = s.validate().unwrap_err();
    assert!(
        err.to_string().contains("rtt_contains cannot be empty"),
        "unexpected error: {err}"
    );
}
