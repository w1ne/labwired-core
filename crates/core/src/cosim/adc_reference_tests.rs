//! Raw numerical admission is separate from ADC conversion qualification.
use super::*;
use std::sync::{Arc, Mutex};

const PATH: &str = "board.power.vdda_volts";

#[derive(Debug, Clone, Default, PartialEq)]
struct Observation {
    volts: Option<f64>,
    available: bool,
    setter_calls: usize,
}

#[derive(Debug)]
struct Receiver {
    maximum: Option<f64>,
    reference_support: bool,
    accepts_unpowered: bool,
    observation: Arc<Mutex<Observation>>,
}

impl Peripheral for Receiver {
    fn read(&self, _: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _: u64, _: u8) -> SimResult<()> {
        Ok(())
    }
    fn supports_adc_reference_voltage(&self) -> bool {
        self.reference_support
    }
    fn adc_reference_maximum_voltage_v(&self) -> Option<f64> {
        self.maximum
    }
    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        let mut observation = self.observation.lock().unwrap();
        observation.setter_calls += 1;
        // Deliberately do not repeat the bus numerical-bound check: these
        // tests must detect a router/bus guard that passes an invalid sample.
        if !volts.is_finite() || (!self.accepts_unpowered && volts <= 0.0) {
            return false;
        }
        observation.volts = Some(volts);
        observation.available = volts > 0.0;
        true
    }
}

fn add_receiver(
    bus: &mut SystemBus,
    index: u64,
    maximum: Option<f64>,
    reference_support: bool,
    accepts_unpowered: bool,
) -> Arc<Mutex<Observation>> {
    let observation = Arc::new(Mutex::new(Observation::default()));
    bus.replace_or_add_peripheral(
        &format!("reference_{index}"),
        0x5000_0000 + 0x1000 * index,
        0x400,
        None,
        Box::new(Receiver {
            maximum,
            reference_support,
            accepts_unpowered,
            observation: observation.clone(),
        }),
    );
    observation
}

fn router(bus: &SystemBus) -> SignalRouter {
    let model = CosimModelConfig {
        id: "reference".into(),
        adapter: ManifestCosimAdapter::Mock,
        model: None,
        step_ns: 100,
        inputs: Default::default(),
        outputs: [("vdda".into(), PATH.into())].into_iter().collect(),
        config: Default::default(),
    };
    let (router, errors) = SignalRouter::bind(&[model], bus);
    assert!(errors.is_empty(), "{errors:?}");
    router
}

fn apply(router: &mut SignalRouter, bus: &mut SystemBus, volts: f64) -> Vec<RoutingError> {
    router.apply(
        bus,
        &[(PATH.into(), CosimSignalValue::F64(volts))]
            .into_iter()
            .collect(),
        100,
    )
}

fn assert_invalid(errors: &[RoutingError], volts: f64) {
    match errors {
        [RoutingError::InvalidAdcReference {
            path,
            volts: observed,
        }] => {
            assert_eq!(path, PATH);
            assert_eq!(observed.to_bits(), volts.to_bits());
        }
        other => panic!("expected raw ADC admission refusal, got {other:?}"),
    }
}

#[test]
fn declared_reference_preserves_exact_numerical_zero_and_remains_unpowered() {
    let mut bus = SystemBus::new();
    let state = add_receiver(&mut bus, 0, Some(4.0), true, true);
    let mut router = router(&bus);
    for volts in [-f64::EPSILON * 4.0, -3.3382408854924333e-19, -0.0, 0.0] {
        assert!(apply(&mut router, &mut bus, volts).is_empty());
        let observation = state.lock().unwrap();
        assert_eq!(observation.volts.unwrap().to_bits(), volts.to_bits());
        assert!(!observation.available);
    }
    assert!(apply(&mut router, &mut bus, 3.3).is_empty());
    assert!(state.lock().unwrap().available);
}

#[test]
fn next_value_beyond_declared_zero_bound_refuses_before_any_setter() {
    let mut bus = SystemBus::new();
    let state = add_receiver(&mut bus, 0, Some(4.0), true, true);
    assert!(bus.seed_adc_reference_voltage(3.3));
    let before = state.lock().unwrap().clone();
    let boundary = -f64::EPSILON * 4.0;
    let beyond = f64::from_bits(boundary.to_bits() + 1);
    assert!(beyond < boundary);
    let mut router = router(&bus);
    assert_invalid(&apply(&mut router, &mut bus, beyond), beyond);
    assert!(!bus.seed_adc_reference_voltage(beyond));
    assert_eq!(*state.lock().unwrap(), before);
}

#[test]
fn nonfinite_and_material_negative_references_refuse_without_mutation() {
    let mut bus = SystemBus::new();
    let state = add_receiver(&mut bus, 0, Some(4.0), true, true);
    assert!(bus.seed_adc_reference_voltage(3.3));
    let before = state.lock().unwrap().clone();
    let mut router = router(&bus);
    for volts in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.001, -1.0] {
        assert_invalid(&apply(&mut router, &mut bus, volts), volts);
        assert!(!bus.seed_adc_reference_voltage(volts));
        assert_eq!(*state.lock().unwrap(), before);
    }
}

#[test]
fn invalid_declared_maxima_fail_closed_even_for_positive_samples() {
    for maximum in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        let mut bus = SystemBus::new();
        let state = add_receiver(&mut bus, 0, Some(maximum), true, true);
        let mut router = router(&bus);
        assert_invalid(&apply(&mut router, &mut bus, 3.3), 3.3);
        assert!(!bus.seed_adc_reference_voltage(3.3));
        assert_eq!(*state.lock().unwrap(), Observation::default());
    }
}

#[test]
fn absent_maximum_preserves_legacy_nonnegative_admission() {
    let mut bus = SystemBus::new();
    let state = add_receiver(&mut bus, 0, None, true, true);
    let mut router = router(&bus);
    let tiny_negative = -f64::from_bits(1);
    assert_invalid(&apply(&mut router, &mut bus, tiny_negative), tiny_negative);
    assert_eq!(*state.lock().unwrap(), Observation::default());
    for volts in [-0.0, 0.0, 3.3] {
        assert!(apply(&mut router, &mut bus, volts).is_empty());
        assert_eq!(
            state.lock().unwrap().volts.unwrap().to_bits(),
            volts.to_bits()
        );
    }
}

#[test]
fn every_reference_consumer_is_preflighted_independent_of_bus_order() {
    for maxima in [[Some(4.0), None], [None, Some(4.0)]] {
        let mut bus = SystemBus::new();
        let a = add_receiver(&mut bus, 0, maxima[0], true, true);
        let b = add_receiver(&mut bus, 1, maxima[1], true, true);
        assert!(bus.seed_adc_reference_voltage(3.3));
        let before = [a.lock().unwrap().clone(), b.lock().unwrap().clone()];
        let volts = -f64::EPSILON;
        let mut router = router(&bus);
        assert_invalid(&apply(&mut router, &mut bus, volts), volts);
        assert!(!bus.seed_adc_reference_voltage(volts));
        assert_eq!(*a.lock().unwrap(), before[0]);
        assert_eq!(*b.lock().unwrap(), before[1]);
    }
}

#[test]
fn shared_reference_uses_each_receivers_own_maximum() {
    let mut bus = SystemBus::new();
    let a = add_receiver(&mut bus, 0, Some(4.0), true, true);
    let b = add_receiver(&mut bus, 1, Some(2.0), true, true);
    let mut router = router(&bus);
    let too_negative = -f64::EPSILON * 4.0;
    assert_invalid(&apply(&mut router, &mut bus, too_negative), too_negative);
    assert_eq!(*a.lock().unwrap(), Observation::default());
    assert_eq!(*b.lock().unwrap(), Observation::default());
    let accepted = -f64::EPSILON * 2.0;
    assert!(apply(&mut router, &mut bus, accepted).is_empty());
    for state in [a, b] {
        let observation = state.lock().unwrap();
        assert_eq!(observation.volts.unwrap().to_bits(), accepted.to_bits());
        assert!(!observation.available);
    }
}

#[test]
fn current_receivers_are_checked_after_router_binding() {
    let mut bus = SystemBus::new();
    let a = add_receiver(&mut bus, 0, Some(4.0), true, true);
    let mut router = router(&bus);
    let b = add_receiver(&mut bus, 1, None, true, true);
    let volts = -f64::EPSILON;
    assert_invalid(&apply(&mut router, &mut bus, volts), volts);
    assert_eq!(*a.lock().unwrap(), Observation::default());
    assert_eq!(*b.lock().unwrap(), Observation::default());
}

#[test]
fn unrelated_peripherals_do_not_supply_or_veto_the_adc_maximum() {
    let mut bus = SystemBus::new();
    let unrelated = add_receiver(&mut bus, 0, Some(f64::NAN), false, true);
    let reference = add_receiver(&mut bus, 1, Some(4.0), true, true);
    let mut router = router(&bus);
    assert!(apply(&mut router, &mut bus, -f64::EPSILON).is_empty());
    assert_eq!(*unrelated.lock().unwrap(), Observation::default());
    assert_eq!(reference.lock().unwrap().setter_calls, 1);
}

#[test]
fn typed_reference_inputs_are_required_without_mutation() {
    let mut bus = SystemBus::new();
    let state = add_receiver(&mut bus, 0, Some(4.0), true, true);
    let mut router = router(&bus);
    for value in [
        CosimSignalValue::Bool(false),
        CosimSignalValue::Text("0".into()),
    ] {
        let signals = [(PATH.into(), value.clone())].into_iter().collect();
        assert_eq!(
            router.apply(&mut bus, &signals, 100),
            vec![RoutingError::TypeMismatch {
                path: PATH.into(),
                value
            }]
        );
        assert_eq!(*state.lock().unwrap(), Observation::default());
    }
}

#[test]
fn numerical_admission_does_not_override_model_unpowered_refusal() {
    let mut bus = SystemBus::new();
    let state = add_receiver(&mut bus, 0, Some(4.0), true, false);
    let mut router = router(&bus);
    assert!(apply(&mut router, &mut bus, 3.3).is_empty());
    for volts in [0.0, -f64::EPSILON] {
        assert_eq!(
            apply(&mut router, &mut bus, volts),
            vec![RoutingError::AdcReferenceRejected {
                path: PATH.into(),
                volts
            }]
        );
        let observation = state.lock().unwrap();
        assert_eq!(observation.volts, Some(3.3));
        assert!(observation.available);
    }
}

#[test]
fn legacy_adc_still_requires_positive_reference_and_keeps_its_scale() {
    let mut adc = crate::peripherals::adc::Adc::new();
    assert_eq!(adc.adc_reference_maximum_voltage_v(), None);
    assert!(adc.set_adc_reference_voltage_v(1.65));
    assert!(!adc.set_adc_reference_voltage_v(0.0));
    assert!(!adc.set_adc_reference_voltage_v(-f64::EPSILON));
    assert_eq!(adc.reference_voltage_v(), 1.65);
}

#[test]
fn no_reference_consumer_does_not_accept_an_observation() {
    let mut bus = SystemBus::new();
    assert!(!bus.adc_reference_voltage_is_admissible(3.3));
    assert!(!bus.seed_adc_reference_voltage(3.3));
}
