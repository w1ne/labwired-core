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
    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts < 0.0 || (volts == 0.0 && !self.accepts_unpowered) {
            return false;
        }
        *self.reference.lock().unwrap() = volts;
        true
    }
}

fn fixture(zero_support: &[bool]) -> (SystemBus, SignalRouter, Vec<Arc<Mutex<f64>>>) {
    let mut bus = SystemBus::new();
    let mut references = Vec::new();
    for (index, accepts_unpowered) in zero_support.iter().enumerate() {
        let reference = Arc::new(Mutex::new(3.3));
        bus.add_peripheral(
            &format!("adc{index}"),
            0x5000_0000 + index as u64 * 0x100,
            0x100,
            None,
            Box::new(ReferenceAdc {
                reference: reference.clone(),
                accepts_unpowered: *accepts_unpowered,
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
