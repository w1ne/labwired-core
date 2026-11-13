mod controlled_buck_support;
use controlled_buck_support::*;
use labwired_core::regulator::{RegulatorObservation, RegulatorRegistry};
fn observation(time: u64, revision: u64) -> RegulatorObservation {
    RegulatorObservation {
        time_ns: time,
        input_voltage_v: 3.3,
        feedback_voltage_v: 1.1,
        input_current_a: 0.04,
        output_current_a: 0.1,
        output_enabled: true,
        qualified: true,
        applied_revision: revision,
    }
}
#[test]
fn no_stage_means_no_state_changing_command_or_invented_observation() {
    let registry = RegulatorRegistry::new(&[descriptor()]);
    let control = &registry.get("converter").unwrap().control;
    assert!(control.observation().is_none());
    assert!(!control.pending_update());
    let mut next = control.configuration();
    next.enabled = false;
    assert!(control.request(next).is_err());
    assert!(registry
        .apply_observations(&[("converter".into(), observation(0, 0))])
        .is_err());
    assert!(control.request(control.configuration()).is_ok());
}
#[test]
fn queued_commands_are_ordered_and_acknowledge_only_exported_revisions() {
    let registry = RegulatorRegistry::new(&[descriptor()]);
    registry
        .attach_sampled_sources(&["converter".into()])
        .unwrap();
    let control = &registry.get("converter").unwrap().control;
    let mut next = control.configuration();
    next.target_voltage_v = 1.2;
    control.request(next).unwrap();
    assert_eq!(control.command().command_revision, 1);
    next.target_voltage_v = 1.3;
    control.request(next).unwrap();
    assert_eq!(control.configuration(), next);
    assert_eq!(control.command().command_revision, 1);
    assert!(registry
        .apply_observations(&[("converter".into(), observation(1, 2))])
        .is_err());
    registry
        .apply_observations(&[("converter".into(), observation(1, 1))])
        .unwrap();
    assert!(control.pending_update());
    assert_eq!(control.command().command_revision, 2);
    registry
        .apply_observations(&[("converter".into(), observation(2, 1))])
        .unwrap();
    assert!(control.pending_update());
    registry
        .apply_observations(&[("converter".into(), observation(3, 2))])
        .unwrap();
    assert!(!control.pending_update());
}
#[test]
fn reset_clears_queued_hiz_but_retains_physical_observation_and_inflight_identity() {
    let registry = RegulatorRegistry::new(&[descriptor()]);
    registry
        .attach_sampled_sources(&["converter".into()])
        .unwrap();
    let control = &registry.get("converter").unwrap().control;
    registry
        .apply_observations(&[("converter".into(), observation(0, 0))])
        .unwrap();
    let mut off = control.configuration();
    off.enabled = false;
    control.request(off).unwrap();
    assert_eq!(control.command().command_revision, 1);
    control.reset_to_defaults().unwrap();
    assert!(control.configuration().enabled);
    assert_eq!(control.observation().unwrap().time_ns, 0);
    let mut old = observation(1, 1);
    old.output_enabled = false;
    registry
        .apply_observations(&[("converter".into(), old)])
        .unwrap();
    assert!(control.pending_update());
    assert_eq!(control.command().command_revision, 2);
    registry
        .apply_observations(&[("converter".into(), observation(2, 2))])
        .unwrap();
    assert!(!control.pending_update());
}
#[test]
fn malformed_peer_batch_cannot_publish_half_an_observation() {
    let first = descriptor();
    let mut second = first.clone();
    second.id = "other".into();
    let registry = RegulatorRegistry::new(&[first, second]);
    registry
        .attach_sampled_sources(&["converter".into(), "other".into()])
        .unwrap();
    let mut bad = observation(0, 0);
    bad.input_voltage_v = f64::NAN;
    assert!(registry
        .apply_observations(&[
            ("converter".into(), observation(0, 0)),
            ("other".into(), bad)
        ])
        .is_err());
    assert!(registry
        .get("converter")
        .unwrap()
        .control
        .observation()
        .is_none());
    assert!(registry
        .apply_observations(&[
            ("converter".into(), observation(0, 0)),
            ("converter".into(), observation(0, 0))
        ])
        .is_err());
}
#[test]
fn bounded_queue_refuses_overflow_without_changing_accepted_configuration() {
    let registry = RegulatorRegistry::new(&[descriptor()]);
    registry
        .attach_sampled_sources(&["converter".into()])
        .unwrap();
    let control = &registry.get("converter").unwrap().control;
    for index in 0..32 {
        let mut next = control.configuration();
        next.target_voltage_v = 1.2 + f64::from(index) * 0.01;
        control.request(next).unwrap();
    }
    let before = control.configuration();
    let mut next = before;
    next.enabled = false;
    assert!(control.request(next).is_err());
    assert_eq!(control.configuration(), before);
}
