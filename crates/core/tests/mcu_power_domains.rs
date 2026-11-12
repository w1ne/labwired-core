mod mcu_power_support;
use labwired_core::power_domains::{McuPowerDomains, PowerResetCause};

fn model() -> McuPowerDomains {
    McuPowerDomains::new(Some(mcu_power_support::config())).unwrap()
}
fn attach(model: &mut McuPowerDomains) {
    model
        .attach_sampled_routes(
            &[
                "core".into(),
                "io".into(),
                "detector".into(),
                "flash".into(),
            ],
            1000,
        )
        .unwrap();
}
fn sample(model: &mut McuPowerDomains, time: u64, core: f64, detector: f64) {
    model
        .apply_voltages(
            &[
                ("core".into(), core),
                ("io".into(), 3.3),
                ("detector".into(), detector),
                ("flash".into(), 3.3),
            ],
            time,
        )
        .unwrap();
}
fn boot(model: &mut McuPowerDomains) {
    attach(model);
    sample(model, 0, 1.1, 3.3);
    sample(model, 3000, 1.1, 3.3);
    assert_eq!(model.take_reset_event(), Some(PowerResetCause::PowerOn));
    assert!(model.needs_restart());
    model.finish_restart();
}

#[test]
fn one_routed_domain_makes_every_unobserved_peer_unavailable() {
    let mut model = model();
    let io = model.handle("io").unwrap();
    let core = model.handle("core").unwrap();
    assert!(io.is_available());
    assert_eq!(io.effective_voltage_v(), Some(3.3));
    model.attach_sampled_routes(&["core".into()], 1000).unwrap();
    assert!(!io.is_available());
    assert_eq!(io.effective_voltage_v(), None);
    assert!(!io.observation().routed);
    assert!(io.observation().measured_mode);
    model.apply_voltages(&[("core".into(), 1.1)], 0).unwrap();
    assert!(core.is_available());
    assert!(!io.is_available());
    assert!(model.blocks_execution());
    assert_eq!(
        model.take_reset_event(),
        None,
        "attaching a route is not a reset event"
    );
}

#[test]
fn malformed_rail_batches_do_not_half_apply_or_create_nominals() {
    let mut model = model();
    attach(&mut model);
    sample(&mut model, 0, 1.1, 3.3);
    let old = model.handle("io").unwrap().observation();
    for bad in [
        vec![("io".into(), 0.0), ("missing".into(), 3.3)],
        vec![("io".into(), 0.0), ("core".into(), f64::NAN)],
        vec![("io".into(), 0.0), ("io".into(), 3.3)],
        vec![("io".into(), -1.0)],
    ] {
        assert!(model.apply_voltages(&bad, 1000).is_err());
        assert_eq!(model.handle("io").unwrap().observation(), old);
    }
    sample(&mut model, 2000, 1.1, 3.3);
    assert!(model.apply_voltages(&[], 1000).is_err());
    assert!(model
        .attach_sampled_routes(&["missing".into()], 1000)
        .is_err());
    assert!(model.attach_sampled_routes(&["io".into()], 2000).is_err());
}

#[test]
fn por_latch_and_operating_qualification_are_distinct() {
    let mut model = model();
    attach(&mut model);
    sample(&mut model, 0, 0.97, 3.3);
    sample(&mut model, 2999, 0.97, 3.3);
    assert!(model.is_held());
    sample(&mut model, 3000, 0.97, 3.3);
    assert!(!model.is_held());
    assert!(!model.execution_qualified());
    assert!(!model.needs_restart());
    assert_eq!(model.take_reset_event(), Some(PowerResetCause::PowerOn));
    sample(&mut model, 4000, 1.1, 3.3);
    assert!(model.needs_restart());
    model.finish_restart();
    sample(&mut model, 5000, 0.90, 3.3);
    assert!(
        !model.is_held(),
        "DVDD falling does not rearm a latched POR"
    );
    assert!(!model.execution_qualified());
    sample(&mut model, 6000, 1.1, 0.0);
    assert!(model.is_held());
    assert_eq!(model.take_reset_event(), Some(PowerResetCause::PowerOn));
    sample(&mut model, 7000, 1.1, 3.3);
    sample(&mut model, 10000, 1.1, 3.3);
    assert!(!model.is_held());
    assert_eq!(
        model.take_reset_event(),
        None,
        "release cannot count the assertion twice"
    );
}

#[test]
fn detector_operating_failure_blocks_qualification_without_inventing_a_reset() {
    let mut model = model();
    boot(&mut model);
    sample(&mut model, 4000, 1.1, 2.0);
    assert!(!model.is_held());
    assert!(!model.execution_qualified());
    assert_eq!(model.take_reset_event(), None);
    assert!(!model.status().unwrap().supervision_qualified);
}

#[test]
fn brown_out_delays_are_sampled_and_bor_reactivation_uses_startup_delay() {
    let mut model = model();
    boot(&mut model);
    sample(&mut model, 57000, 0.90, 3.3);
    assert!(!model.is_held());
    sample(&mut model, 58000, 0.90, 3.3);
    sample(&mut model, 60999, 0.90, 3.3);
    assert!(!model.is_held());
    sample(&mut model, 61000, 0.90, 3.3);
    assert!(model.is_held());
    assert_eq!(model.take_reset_event(), Some(PowerResetCause::BrownOut));
    assert_eq!(
        model.status().unwrap().brown_out.unwrap().active_at_ns,
        Some(116000)
    );
    sample(&mut model, 62000, 1.1, 3.3);
    sample(&mut model, 116999, 1.1, 3.3);
    assert!(model.is_held());
    sample(&mut model, 117000, 1.1, 3.3);
    assert!(!model.is_held());
    assert!(model.needs_restart());
    assert_eq!(model.take_reset_event(), None);
}

#[test]
fn programmable_control_needs_a_sampled_source_and_never_drops_requests() {
    let mut model = model();
    let control = model.supervisor_control().unwrap();
    let defaults = control.brown_out_configuration();
    control
        .configure_brown_out(defaults.enabled, defaults.falling_v, defaults.rising_v)
        .unwrap();
    assert!(control
        .configure_brown_out(false, defaults.falling_v, defaults.rising_v)
        .is_err());
    boot(&mut model);
    control.configure_brown_out(false, 0.90, 0.91).unwrap();
    assert!(model.status().unwrap().brown_out.unwrap().applied.enabled);
    sample(&mut model, 4000, 1.1, 3.3);
    assert!(!model.status().unwrap().brown_out.unwrap().applied.enabled);
    control.configure_brown_out(true, 0.90, 0.91).unwrap();
    sample(&mut model, 5000, 1.1, 3.3);
    assert_eq!(
        model.status().unwrap().brown_out.unwrap().active_at_ns,
        Some(40000)
    );
    control.configure_brown_out(true, 0.91, 0.92).unwrap();
    sample(&mut model, 6000, 1.1, 3.3);
    assert_eq!(
        model.status().unwrap().brown_out.unwrap().active_at_ns,
        Some(26000)
    );
    for i in 0..32 {
        control.configure_brown_out(i % 2 == 0, 0.90, 0.91).unwrap();
    }
    let accepted = control.brown_out_configuration();
    assert!(control
        .configure_brown_out(!accepted.enabled, 0.90, 0.91)
        .is_err());
    assert_eq!(control.brown_out_configuration(), accepted);
    for (low, high) in [(f64::NAN, 1.0), (-1.0, 1.0), (1.1, 1.0)] {
        assert!(control.configure_brown_out(true, low, high).is_err());
    }
}

#[test]
fn pin_reset_restores_accepted_defaults_once_and_restarts_delay_at_boundary() {
    let mut model = model();
    boot(&mut model);
    let control = model.supervisor_control().unwrap();
    control.configure_brown_out(false, 0.80, 0.81).unwrap();
    model.record_reset(PowerResetCause::ExternalPin).unwrap();
    let defaults = control.brown_out_configuration();
    assert!(defaults.enabled);
    assert_eq!(defaults.falling_v, 0.946);
    assert_eq!(
        model.status().unwrap().brown_out.unwrap().pending_changes,
        0
    );
    assert_eq!(model.take_reset_event(), Some(PowerResetCause::ExternalPin));
    assert_eq!(model.take_reset_event(), None);
    sample(&mut model, 4000, 1.1, 3.3);
    assert_eq!(
        model.status().unwrap().brown_out.unwrap().active_at_ns,
        Some(39000)
    );
    model.finish_restart();
    assert!(!model.needs_restart());
    assert_eq!(model.status().unwrap().reset_events, 2);
}

#[test]
fn identical_sample_sequences_ignore_host_observation_chunking() {
    let mut a = model();
    let mut b = model();
    boot(&mut a);
    boot(&mut b);
    for time in (4000..=80000).step_by(1000) {
        let volts = if time < 60000 { 1.1 } else { 0.90 };
        sample(&mut a, time, volts, 3.3);
        for _ in 0..10 {
            let _ = b.status();
            let _ = b.handle("core").unwrap().observation();
        }
        sample(&mut b, time, volts, 3.3);
    }
    assert_eq!(
        serde_json::to_value(a.status()).unwrap(),
        serde_json::to_value(b.status()).unwrap()
    );
}

#[test]
fn mcu_rail_roundoff_retains_raw_zero_boundary_without_qualifying_power() {
    let mut model = model();
    attach(&mut model);
    let domains = ["core", "io", "detector", "flash"];
    for (step, factor) in [-1.0, -0.0, 0.0].into_iter().enumerate() {
        let values: Vec<_> = domains
            .iter()
            .map(|domain| {
                let max = model.handle(domain).unwrap().observation().max_v;
                ((*domain).to_string(), factor * f64::EPSILON * max)
            })
            .collect();
        model.apply_voltages(&values, step as u64 * 1000).unwrap();
        for (domain, raw) in &values {
            let handle = model.handle(domain).unwrap();
            let observed = handle.observation();
            assert_eq!(observed.voltage_v.unwrap().to_bits(), raw.to_bits());
            assert_eq!(observed.within_operating_range, Some(false));
            assert!(!handle.is_available());
            assert_eq!(
                handle.effective_voltage_v().unwrap().to_bits(),
                raw.to_bits()
            );
        }
        assert!(model.blocks_execution());
        assert!(model.is_held());
    }
}

#[test]
fn mcu_rail_roundoff_rejects_the_next_value_and_preserves_the_entire_batch() {
    let mut model = model();
    boot(&mut model);
    let domains = ["core", "io", "detector", "flash"];
    let before = domains.map(|domain| model.handle(domain).unwrap().observation());
    let floor = -f64::EPSILON * 1.16;
    for bad in [
        f64::from_bits(floor.to_bits() + 1),
        -f64::EPSILON * 3.63, // Another rail's larger declared maximum is not valid here.
        -1.0e-9,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert!(model
            .apply_voltages(&[("io".into(), 0.0), ("core".into(), bad)], 4000)
            .is_err());
        for (domain, expected) in domains.into_iter().zip(&before) {
            assert_eq!(&model.handle(domain).unwrap().observation(), expected);
        }
        assert!(!model.is_held());
        assert!(model.execution_qualified());
    }
    // Rejected batches must not consume the observation timestamp either.
    sample(&mut model, 3500, 1.1, 3.3);
}

fn state_json(model: &McuPowerDomains) -> serde_json::Value {
    serde_json::to_value(model.status().unwrap()).unwrap()
}
fn complete_values(core: f64, detector: f64) -> Vec<(String, f64)> {
    vec![
        ("core".into(), core),
        ("io".into(), 3.3),
        ("detector".into(), detector),
        ("flash".into(), 3.3),
    ]
}

#[test]
fn rejected_por_deadline_preserves_rails_latch_events_and_timestamp() {
    let mut model = model();
    attach(&mut model);
    sample(&mut model, 0, 1.1, 3.3);
    let before = state_json(&model);
    let error = model
        .apply_voltages(&complete_values(1.1, 3.3), u64::MAX)
        .unwrap_err();
    assert!(error.contains("time overflow"), "{error}");
    assert_eq!(state_json(&model), before);
    assert_eq!(model.take_reset_event(), None);

    sample(&mut model, 3000, 1.1, 3.3);
    assert_eq!(model.take_reset_event(), Some(PowerResetCause::PowerOn));
    assert_eq!(model.take_reset_event(), None);
}

#[test]
fn rejected_control_deadline_retains_the_pending_command_and_live_handle() {
    let mut model = model();
    boot(&mut model);
    let handle = model.handle("core").unwrap();
    let control = model.supervisor_control().unwrap();
    control.configure_brown_out(false, 0.93, 0.96).unwrap();
    let before = state_json(&model);
    assert_eq!(before["brown_out"]["pending_changes"], 1);
    let error = model
        .apply_voltages(&complete_values(1.09, 3.3), u64::MAX)
        .unwrap_err();
    assert!(error.contains("time overflow"), "{error}");
    assert_eq!(state_json(&model), before);
    assert_eq!(model.take_reset_event(), None);
    assert_eq!(handle.observation().voltage_v, Some(1.1));
    assert!(!control.brown_out_configuration().enabled);

    sample(&mut model, 4000, 1.09, 3.3);
    let after = state_json(&model);
    assert_eq!(after["brown_out"]["pending_changes"], 0);
    assert_eq!(after["brown_out"]["applied"]["enabled"], false);
    assert_eq!(handle.observation().voltage_v, Some(1.09));
}

#[test]
fn rejected_reset_enqueue_restores_consumed_control_and_all_runtime_state() {
    let mut model = model();
    boot(&mut model);
    for _ in 0..32 {
        model.record_reset(PowerResetCause::ExternalPin).unwrap();
    }
    assert!(model.record_reset(PowerResetCause::ExternalPin).is_err());
    let control = model.supervisor_control().unwrap();
    control.configure_brown_out(false, 0.93, 0.96).unwrap();
    let before = state_json(&model);
    assert_eq!(before["brown_out"]["pending_changes"], 1);
    let error = model
        .apply_voltages(&complete_values(1.09, 0.0), 4000)
        .unwrap_err();
    assert!(error.contains("reset event queue is full"), "{error}");
    assert_eq!(state_json(&model), before);
    assert!(!control.brown_out_configuration().enabled);
    for _ in 0..32 {
        assert_eq!(model.take_reset_event(), Some(PowerResetCause::ExternalPin));
    }
    assert_eq!(model.take_reset_event(), None);

    sample(&mut model, 4000, 1.09, 0.0);
    assert_eq!(model.take_reset_event(), Some(PowerResetCause::PowerOn));
    assert_eq!(model.take_reset_event(), None);
    assert!(model.is_held());
}
