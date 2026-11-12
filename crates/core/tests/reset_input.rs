use labwired_config::ResetInputConfig;
use labwired_core::reset_input::ResetInputState;

fn reset() -> ResetInputState {
    let config: ResetInputConfig = serde_yaml::from_str("pin: NRST\nactive_low: true\nvil_max_v: 0.72\nvih_min_v: 2.0\nmin_assert_ns: 80000\ninvalid_pulse_ns: 40000\n").unwrap();
    let mut reset = ResetInputState::new(Some(config));
    reset.attach();
    assert!(!reset.take_assertion());
    reset.apply(3.3, Some(3.3), 0).unwrap();
    assert!(!reset.is_held());
    reset
}

#[test]
fn absolute_reset_limits_do_not_scale_with_a_falling_supply() {
    let mut r = reset();
    r.apply(0.7, Some(1.8), 1_000).unwrap();
    r.apply(0.7, Some(1.8), 81_000).unwrap();
    assert!(r.is_held());
    assert!(r.take_assertion());
    r.apply(1.2, Some(1.8), 82_000).unwrap();
    assert!(
        r.is_held(),
        "1.2V is below the absolute2.0V release threshold even at VDD1.8V"
    );
    r.apply(2.0, Some(1.8), 83_000).unwrap();
    assert!(!r.is_held());
}

#[test]
fn invalid_valid_and_indeterminate_pulse_widths_are_distinct() {
    let mut r = reset();
    r.apply(0.0, Some(3.3), 1_000).unwrap();
    r.apply(3.3, Some(3.3), 41_000).unwrap();
    assert!(!r.take_assertion(), "40us low is guaranteed invalid");
    r.apply(0.0, Some(3.3), 50_000).unwrap();
    r.apply(0.0, Some(3.3), 130_000).unwrap();
    assert!(r.is_held());
    assert!(r.take_assertion(), "80us low is guaranteed valid");
    r.apply(3.3, Some(3.3), 140_000).unwrap();
    assert!(!r.is_held());
    r.apply(0.0, Some(3.3), 150_000).unwrap();
    assert!(r
        .apply(3.3, Some(3.3), 210_000)
        .unwrap_err()
        .contains("indeterminate"));
}

#[test]
fn ratio_reset_limits_use_the_current_rails_and_validate_zero_supply() {
    let config: ResetInputConfig = serde_yaml::from_str("pin: RESET_N\nactive_low: true\nvil: 0.3\nvih: 0.7\nmin_assert_ns: 0\ninvalid_pulse_ns: 0\n").unwrap();
    let mut r = ResetInputState::new(Some(config));
    r.attach();
    r.take_assertion();
    r.apply(1.8, Some(1.8), 0).unwrap();
    assert!(!r.is_held());
    r.apply(0.0, Some(0.0), 1).unwrap();
    assert!(r.is_held());
    r.apply(1.2, Some(1.8), 2).unwrap();
    assert!(r.is_held());
    r.apply(1.3, Some(1.8), 3).unwrap();
    assert!(!r.is_held());
}

#[test]
fn route_attachment_and_an_unknown_initial_level_do_not_invent_reset_causes() {
    let config: ResetInputConfig = serde_yaml::from_str("pin: NRST\nactive_low: true\nvil_max_v: 0.72\nvih_min_v: 2.0\nmin_assert_ns: 80000\ninvalid_pulse_ns: 40000\n").unwrap();
    let mut r = ResetInputState::new(Some(config));
    r.attach();
    assert!(r.is_held());
    assert!(!r.take_assertion());
    r.apply(1.2, Some(3.3), 0).unwrap();
    r.apply(1.2, Some(3.3), 100_000).unwrap();
    assert!(
        r.is_held(),
        "the Schmitt band does not establish an initial high"
    );
    assert!(
        !r.take_assertion(),
        "an unknown level is not a qualified low pulse"
    );
    r.apply(3.3, Some(3.3), 101_000).unwrap();
    assert!(!r.is_held());
    assert_eq!(r.status().unwrap().assertions, 0);
}

#[test]
fn a_valid_sampled_pulse_resets_once_when_its_release_is_the_next_observation() {
    let mut r = reset();
    r.apply(0.0, Some(3.3), 1_000).unwrap();
    assert!(!r.is_held());
    r.apply(3.3, Some(3.3), 101_000).unwrap();
    assert!(!r.is_held());
    assert!(
        r.take_assertion(),
        "the observed low lasted a guaranteed-valid 100 us"
    );
    assert!(!r.take_assertion());
    assert_eq!(r.status().unwrap().assertions, 1);
}
