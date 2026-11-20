use labwired_core::peripherals::components::declarative_gpio::DeclarativeGpioKit;

const VALID: &str = r#"
type: arbitrary-matrix
behavior:
  primitive: gpio_device
  pins:
    clock: clock_pin
    rows: [r0_pin, r1_pin, r2_pin]
    cols: { config: column_pins, count: 5 }
  pin_defaults: { rows: true }
  outputs: ["cols[4]"]
  rules:
    - on: { pins: ["rows[0]", "rows[2]", clock] }
      do: [{ pin: "cols[4]", level: "pin(rows[2]) || pin(clock)" }]
"#;

#[test]
fn mixed_scalar_and_arbitrary_sized_groups_validate() {
    DeclarativeGpioKit::from_yaml(VALID).expect("three rows and five columns are generic groups");
}

#[test]
fn malformed_pin_bindings_fail_preflight() {
    for (from, to, diagnostic) in [
        ("[r0_pin, r1_pin, r2_pin]", "[]", "empty list"),
        (
            "[r0_pin, r1_pin, r2_pin]",
            "[r0_pin, ' ', r2_pin]",
            "blank config key",
        ),
        ("count: 5", "count: 0", "empty list"),
        (
            "clock: clock_pin",
            "'rows[0]': clock_pin",
            "duplicates pin role",
        ),
        (
            "pin_defaults: { rows: true }",
            "pin_defaults: { absent: true }",
            "undeclared role",
        ),
    ] {
        let err = match DeclarativeGpioKit::from_yaml(&VALID.replace(from, to)) {
            Ok(_) => panic!("invalid descriptor accepted: {to}"),
            Err(err) => err,
        };
        assert!(format!("{err:#}").contains(diagnostic), "{to}: {err:#}");
    }
}

#[test]
fn out_of_bounds_events_actions_and_expressions_fail_preflight() {
    for (from, to, diagnostic) in [
        (
            "pins: [\"rows[0]\", \"rows[2]\", clock]",
            "pins: [\"rows[3]\", clock]",
            "rows[3]",
        ),
        ("pin: \"cols[4]\"", "pin: \"cols[5]\"", "cols[5]"),
        ("pin(rows[2])", "pin(rows[3])", "rows[3]"),
        ("pin(rows[2])", "pin(rows[-1])", "rows[-1]"),
    ] {
        let err = match DeclarativeGpioKit::from_yaml(&VALID.replace(from, to)) {
            Ok(_) => panic!("invalid descriptor accepted: {to}"),
            Err(err) => err,
        };
        assert!(format!("{err:#}").contains(diagnostic), "{to}: {err:#}");
    }
}
