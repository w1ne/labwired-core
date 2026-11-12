//! Generic voltage routing capability checks. These ADC test doubles declare
//! whether an unpowered reference is supported; they are not silicon models.
use labwired_config::CosimModelConfig;
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSignalValue, CosimSignals, RoutingError, SignalRouter};
use labwired_core::{Peripheral, SimResult};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct ReferenceAdc {
    reference: Arc<Mutex<f64>>,
    accepts_unpowered: bool,
    observation_max_v: Option<f64>,
}
impl Peripheral for ReferenceAdc {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }
    fn adc_reference_observation_max_v(&self) -> Option<f64> {
        self.observation_max_v
    }
    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        // The test double deliberately leaves the declared roundoff bound to
        // the router, so a routing regression cannot hide in a second check.
        if !volts.is_finite()
            || (volts < 0.0 && self.observation_max_v.is_none())
            || (volts <= 0.0 && !self.accepts_unpowered)
        {
            return false;
        }
        *self.reference.lock().unwrap() = volts;
        true
    }
}

fn fixture(zero_support: &[bool]) -> (SystemBus, SignalRouter, Vec<Arc<Mutex<f64>>>) {
    fixture_with_maxima(
        &zero_support
            .iter()
            .map(|supported| (*supported, None))
            .collect::<Vec<_>>(),
    )
}

fn fixture_with_maxima(
    receivers: &[(bool, Option<f64>)],
) -> (SystemBus, SignalRouter, Vec<Arc<Mutex<f64>>>) {
    let mut bus = SystemBus::new();
    let mut references = Vec::new();
    for (index, (accepts_unpowered, observation_max_v)) in receivers.iter().enumerate() {
        let reference = Arc::new(Mutex::new(3.3));
        bus.add_peripheral(
            &format!("adc{index}"),
            0x5000_0000 + index as u64 * 0x100,
            0x100,
            None,
            Box::new(ReferenceAdc {
                reference: reference.clone(),
                accepts_unpowered: *accepts_unpowered,
                observation_max_v: *observation_max_v,
            }),
        );
        references.push(reference);
    }
    let config: CosimModelConfig = serde_yaml::from_str("id: rail\nadapter: mock\nstep_ns: 1000\noutputs: {vdda: board.power.vdda_volts}\nconfig: {outputs: {vdda: 0.0}}\n").unwrap();
    let (router, errors) = SignalRouter::bind(&[config], &bus);
    assert!(errors.is_empty(), "{errors:?}");
    (bus, router, references)
}
fn apply(bus: &mut SystemBus, router: &mut SignalRouter, volts: f64) -> Vec<RoutingError> {
    let mut signals = CosimSignals::new();
    signals.insert(
        "board.power.vdda_volts".to_string(),
        CosimSignalValue::F64(volts),
    );
    router.apply(bus, &signals, 0)
}

#[test]
fn zero_vdda_reaches_models_that_explicitly_support_an_unpowered_reference() {
    let (mut bus, mut router, refs) = fixture(&[true, true]);
    assert!(
        apply(&mut bus, &mut router, 0.0).is_empty(),
        "zero is an unpowered rail, not an invented positive reference"
    );
    assert!(refs
        .iter()
        .all(|reference| *reference.lock().unwrap() == 0.0));
    assert!(apply(&mut bus, &mut router, 2.1).is_empty());
    assert!(refs
        .iter()
        .all(|reference| *reference.lock().unwrap() == 2.1));
    for invalid in [-0.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            apply(&mut bus, &mut router, invalid).as_slice(),
            [RoutingError::InvalidAdcReference { .. }]
        ));
    }
}

#[test]
fn every_reference_capable_adc_must_accept_a_shared_vdda_observation() {
    for supports in [&[false][..], &[true, false][..], &[false, true][..]] {
        let (mut bus, mut router, _) = fixture(supports);
        let errors = apply(&mut bus, &mut router, 0.0);
        assert_eq!(
            errors.len(),
            1,
            "one accepting ADC cannot hide another ADC retaining a stale reference"
        );
        assert!(
            errors[0].to_string().contains("refused ADC reference"),
            "{errors:?}"
        );
    }
}

#[test]
fn declared_reference_roundoff_preserves_raw_samples_and_rejects_real_negative_voltage() {
    let (mut bus, mut router, refs) = fixture_with_maxima(&[(true, Some(3.6)), (true, Some(1.2))]);
    let roundoff = -f64::EPSILON * 1.2;
    assert!(apply(&mut bus, &mut router, roundoff).is_empty());
    for reference in &refs {
        assert_eq!(reference.lock().unwrap().to_bits(), roundoff.to_bits());
    }
    for invalid in [roundoff * 2.0, -0.1, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            apply(&mut bus, &mut router, invalid).as_slice(),
            [RoutingError::InvalidAdcReference { .. }]
        ));
        for reference in &refs {
            assert_eq!(reference.lock().unwrap().to_bits(), roundoff.to_bits());
        }
    }
}

#[test]
fn a_legacy_reference_receiver_rejects_roundoff_before_any_writer() {
    for receivers in [
        [(true, Some(3.6)), (true, None)],
        [(true, None), (true, Some(3.6))],
    ] {
        let (mut bus, mut router, refs) = fixture_with_maxima(&receivers);
        assert!(matches!(
            apply(&mut bus, &mut router, -1e-20).as_slice(),
            [RoutingError::InvalidAdcReference { .. }]
        ));
        assert!(refs
            .iter()
            .all(|reference| *reference.lock().unwrap() == 3.3));
    }
}

#[test]
fn an_absent_or_invalid_reference_scale_never_admits_negative_voltage() {
    for maximum in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        let (mut bus, mut router, refs) = fixture_with_maxima(&[(true, maximum)]);
        assert!(matches!(
            apply(&mut bus, &mut router, -1e-20).as_slice(),
            [RoutingError::InvalidAdcReference { .. }]
        ));
        assert_eq!(*refs[0].lock().unwrap(), 3.3);
    }
}

#[test]
fn affine_gpio_limits_match_datasheet_at_both_operating_supplies() {
    // AT32F421 datasheet Ver2.03, Table32 p44:
    // https://www.arterychip.com/download/DS/DS_AT32F421_V2.03_EN.pdf
    // These are guaranteed input limits, not measured switching thresholds.
    let coefficients =
        serde_yaml::from_str("vil: 0.28\nvih: 0.31\nvil_offset_v: 0.1\nvih_offset_v: 0.8\n")
            .unwrap();
    for (vdd, low, high) in [(2.1, 0.688, 1.451), (3.3, 1.024, 1.823)] {
        let limits = labwired_core::cosim::InputThresholds::from_ratios(vdd, coefficients);
        assert!((limits.vil_volts - low).abs() < 1e-12);
        assert!((limits.vih_volts - high).abs() < 1e-12);
        assert!(!limits.next_level(true, low - 1e-12));
        assert!(limits.next_level(false, high + 1e-12));
        let band = (low + high) * 0.5;
        assert!(!limits.next_level(false, band));
        assert!(limits.next_level(true, band));
    }
    let low_rail = labwired_core::cosim::InputThresholds::from_ratios(2.1, coefficients);
    let high_rail = labwired_core::cosim::InputThresholds::from_ratios(3.3, coefficients);
    assert!(low_rail.next_level(false, 1.6));
    assert!(!high_rail.next_level(false, 1.6));
}
