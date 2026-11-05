//! Generic analog netlists drive the Arty LiteX CAS switch/button inputs.

mod common;
use common::root;
use labwired_config::SystemManifest;
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const CAS_BASE: u64 = 0x6000_6800;
const SWITCHES: u64 = CAS_BASE + 0x04;
const BUTTONS: u64 = CAS_BASE + 0x08;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
    };
    let (mut low, mut high) = (0.0, 1.2);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = (low + high) / 2.0;
    junction_v + current_at_junction_voltage(junction_v) * DIODE_RS_OHMS
}

fn signal_number(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(value)) => *value,
        other => panic!("expected analog voltage at {path}, got {other:?}"),
    }
}

#[test]
fn arty_litex_ntc_and_diode_netlists_drive_cas_gpio_inputs_through_thresholds() {
    let profile_path = root("configs/systems/arty-litex-vexriscv.yaml");
    let mut profile = SystemManifest::from_file(&profile_path).expect("load Arty LiteX profile");
    assert_eq!(profile.name, "arty-litex-vexriscv");
    let analog_path = root("examples/arty-litex-vexriscv/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path)
        .expect("load generic NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = profile_path.parent().unwrap().join(&profile.chip);
    let chip = labwired_config::ChipDescriptor::from_file(&chip_path)
        .expect("load Arty LiteX chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Arty LiteX bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both solved devices must bind to CAS switch/button inputs"
    );

    let diode_voltage = diode_load_line_voltage();
    let mut cycles = 0;
    for (temperature_c, expected_switch_level) in [
        (-10.0_f64, true),
        (25.0, true),
        (80.0, false),
        (25.0, false),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve both analog netlists and drive CAS inputs");
        assert_eq!(steps.len(), 2, "both analog netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_voltage = signal_number(&session, "board.gpio_in.cas.32");
        let expected_ntc_voltage = 3.3 * ntc_resistance_ohms(temperature_c)
            / (10_000.0 + ntc_resistance_ohms(temperature_c));
        assert!(
            (ntc_voltage - expected_ntc_voltage).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_voltage} V, got {ntc_voltage} V"
        );

        let solved_diode_voltage = signal_number(&session, "board.gpio_in.cas.64");
        assert!(
            (solved_diode_voltage - diode_voltage).abs() < 2e-5,
            "the Shockley load line should be {diode_voltage} V, got {solved_diode_voltage} V"
        );

        let switches = bus.read_u32(SWITCHES).expect("read CAS switch status");
        assert_eq!(
            switches & 1 != 0,
            expected_switch_level,
            "SW0 must follow the analog voltage threshold and retain its Schmitt state at 25 °C"
        );
        assert_eq!(
            bus.read_u32(BUTTONS).expect("read CAS button status") & 1,
            0,
            "the diode's solved forward voltage stays below VIL on active-low BTN0"
        );
    }
}
