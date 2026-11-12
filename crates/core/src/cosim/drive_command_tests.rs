//! Commands and external observations have different truth sources.
use super::*;
use crate::pins::{External, InputChange, PadDriver, PinPort};
use std::sync::{Arc, Mutex};

const COMMAND: &str = "board.gpio_drive_volts.pa5";
const ENABLE: &str = "board.gpio_driven.pa5";
const OBSERVATION: &str = "board.gpio_pad_volts.pa5";

#[derive(Debug)]
struct State {
    driver: Option<PadDriver>,
    external: External,
}
#[derive(Debug)]
struct Port(Arc<Mutex<State>>);
impl PinPort for Port {
    fn pin_count(&self) -> u8 {
        16
    }
    fn driver(&self, pin: u8) -> Option<PadDriver> {
        (pin < 16).then(|| self.0.lock().unwrap().driver).flatten()
    }
    fn external(&self, _: u8) -> External {
        self.0.lock().unwrap().external
    }
    fn set_external(&mut self, _: u8, _: External) -> Option<InputChange> {
        None
    }
    fn input(&self, _: u8) -> Option<bool> {
        self.0.lock().unwrap().external.level()
    }
}
impl Peripheral for Port {
    fn read(&self, _: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _: u64, _: u8) -> SimResult<()> {
        Ok(())
    }
    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }
    fn gpio_port_offsets(&self) -> Option<crate::peripherals::gpio::GpioPortOffsets> {
        Some(crate::peripherals::gpio::GpioPortOffsets {
            input: 0x10,
            output: 0x14,
        })
    }
}
fn bus(driver: Option<PadDriver>, external: External) -> (SystemBus, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State { driver, external }));
    let mut bus = SystemBus::new();
    bus.io_voltage_v = Some(3.3);
    bus.replace_or_add_peripheral(
        "gpioa",
        0x4800_0000,
        0x400,
        None,
        Box::new(Port(state.clone())),
    );
    assert_eq!(
        bus.peripherals.iter().filter(|p| p.name == "gpioa").count(),
        1
    );
    let owner = bus.find_peripheral_index_by_name("gpioa").unwrap();
    assert_eq!(bus.peripherals[owner].base, 0x4800_0000);
    bus.pin_map.insert("pa5".into(), ("gpioa".into(), 5));
    bus.pin_map.insert("PA5".into(), ("gpioa".into(), 5));
    (bus, state)
}
fn model(id: &str, paths: &[&str]) -> CosimModelConfig {
    CosimModelConfig {
        id: id.into(),
        adapter: labwired_config::CosimAdapter::Mock,
        model: None,
        step_ns: 100,
        inputs: paths
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("in{i}"), (*p).into()))
            .collect(),
        outputs: Default::default(),
        config: Default::default(),
    }
}
fn router(bus: &SystemBus, observe: bool) -> SignalRouter {
    let mut paths = vec![COMMAND, ENABLE];
    if observe {
        paths.push(OBSERVATION);
    }
    let (router, errors) = SignalRouter::bind(&[model("driver", &paths)], bus);
    assert!(errors.is_empty(), "{errors:?}");
    router
}
fn number(signals: &CosimSignals, path: &str) -> f64 {
    match signals.get(path) {
        Some(CosimSignalValue::F64(value)) => *value,
        other => panic!("{path}: {other:?}"),
    }
}
#[test]
fn command_path_is_read_only_and_keeps_dotted_labels() {
    let path = SignalPath::parse("board.gpio_drive_volts.p0.13").unwrap();
    assert_eq!(
        path,
        SignalPath::GpioDriveVolts {
            pad: "p0.13".into()
        }
    );
    assert!(path.is_readable());
    assert!(!path.is_writable());
    assert_eq!(SignalPath::parse("board.gpio_drive_volts."), None);
}
#[test]
fn disabled_command_does_not_assign_a_floating_pad_level() {
    let (bus, _) = bus(Some(PadDriver::OFF), External::Released);
    let router = router(&bus, true);
    let mut signals = CosimSignals::new();
    router.sample(&bus, &mut signals);
    assert_eq!(number(&signals, COMMAND), 0.0);
    assert_eq!(signals.get(ENABLE), Some(&CosimSignalValue::Bool(false)));
    assert!(!signals.contains_key(OBSERVATION));
    router.require_drive_commands(&signals).unwrap();
}
#[test]
fn disabled_command_preserves_nonzero_external_pad_observation() {
    let (bus, state) = bus(Some(PadDriver::OFF), External::Level(true));
    let router = router(&bus, true);
    let mut signals = CosimSignals::new();
    router.sample(&bus, &mut signals);
    assert_eq!(number(&signals, COMMAND), 0.0);
    assert_eq!(signals.get(ENABLE), Some(&CosimSignalValue::Bool(false)));
    assert_eq!(number(&signals, OBSERVATION), 3.3);
    assert_eq!(state.lock().unwrap().external, External::Level(true));
}
#[test]
fn command_uses_own_output_stage_through_an_external_conflict() {
    let (bus, state) = bus(Some(PadDriver::drive(true)), External::Level(false));
    let router = router(&bus, false);
    let mut signals = CosimSignals::new();
    router.sample(&bus, &mut signals);
    assert_eq!(number(&signals, COMMAND), 3.3);
    assert_eq!(signals.get(ENABLE), Some(&CosimSignalValue::Bool(true)));
    assert_eq!(state.lock().unwrap().external, External::Level(false));
    state.lock().unwrap().driver = Some(PadDriver::drive(false));
    router.sample(&bus, &mut signals);
    assert_eq!(number(&signals, COMMAND), 0.0);
    assert_eq!(signals.get(ENABLE), Some(&CosimSignalValue::Bool(true)));
}
#[test]
fn unknown_driver_removes_old_command_even_with_known_external_level() {
    let (bus, state) = bus(Some(PadDriver::drive(true)), External::Level(true));
    let router = router(&bus, true);
    let mut signals = CosimSignals::new();
    router.sample(&bus, &mut signals);
    assert_eq!(number(&signals, COMMAND), 3.3);
    state.lock().unwrap().driver = None;
    router.sample(&bus, &mut signals);
    assert!(!signals.contains_key(COMMAND));
    assert_eq!(number(&signals, OBSERVATION), 3.3);
    assert!(matches!(
        router.require_drive_commands(&signals),
        Err(RoutingError::UnavailableDriveCommand { .. })
    ));
}
#[test]
fn enabled_command_requires_a_finite_positive_supply() {
    let (mut bus, _) = bus(Some(PadDriver::drive(true)), External::Released);
    let router = router(&bus, false);
    for volts in [
        None,
        Some(0.0),
        Some(-0.1),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        let mut signals = CosimSignals::new();
        signals.insert(COMMAND.into(), CosimSignalValue::F64(3.3));
        bus.io_voltage_v = volts;
        router.sample(&bus, &mut signals);
        assert!(!signals.contains_key(COMMAND), "{volts:?}");
        assert!(router.require_drive_commands(&signals).is_err());
    }
}
#[test]
fn a_drive_command_requires_enable_in_each_owning_model() {
    let (bus, _) = bus(Some(PadDriver::OFF), External::Released);
    for configs in [
        vec![model("voltage", &[COMMAND])],
        vec![model("voltage", &[COMMAND]), model("other", &[ENABLE])],
        vec![model("voltage", &[COMMAND, "board.gpio_driven.pa4"])],
    ] {
        let (_, errors) = SignalRouter::bind(&configs, &bus);
        assert!(
            errors.iter().any(|e| matches!(e,
            RoutingError::MissingDriveEnable { model, .. } if model == "voltage")),
            "{errors:?}"
        );
    }
    let (_, errors) =
        SignalRouter::bind(&[model("both", &[COMMAND, "board.gpio_driven.PA5"])], &bus);
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn an_unknown_pad_is_not_made_into_a_disabled_command() {
    let (bus, _) = bus(Some(PadDriver::OFF), External::Released);
    for pad in ["does_not_exist", "pa99", ""] {
        let command = format!("board.gpio_drive_volts.{pad}");
        let enabled = format!("board.gpio_driven.{pad}");
        let (_, errors) = SignalRouter::bind(&[model("unknown", &[&command, &enabled])], &bus);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, RoutingError::UnknownPad { .. })),
            "{errors:?}"
        );
    }
}
#[test]
fn a_model_cannot_write_a_driver_command() {
    let (bus, _) = bus(Some(PadDriver::OFF), External::Released);
    let mut config = model("bad", &[]);
    config.outputs.insert("value".into(), COMMAND.into());
    let (_, errors) = SignalRouter::bind(&[config], &bus);
    assert!(matches!(
        errors.as_slice(),
        [RoutingError::NotWritable { .. }]
    ));
}
#[test]
fn no_nominal_io_voltage_means_no_command_binding() {
    let (mut bus, _) = bus(Some(PadDriver::OFF), External::Released);
    bus.io_voltage_v = None;
    let (_, errors) = SignalRouter::bind(&[model("bad", &[COMMAND, ENABLE])], &bus);
    assert!(errors.iter().any(|e| matches!(
        e,
        RoutingError::NoInputThresholds {
            key: "io_voltage_v",
            ..
        }
    )));
}

#[test]
fn missing_command_refuses_before_a_standalone_model_advances() {
    use crate::cosim::{CosimAdapter as RuntimeAdapter, CosimStep, CosimStepResult};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counter(Arc<AtomicUsize>);
    impl RuntimeAdapter for Counter {
        fn step(&mut self, _: CosimStep) -> SimResult<CosimStepResult> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(CosimStepResult::default())
        }
    }
    let (mut bus, state) = bus(Some(PadDriver::drive(true)), External::Level(true));
    bus.cpu_hz = 1_000_000;
    let calls = Arc::new(AtomicUsize::new(0));
    let config = model("command_consumer", &[COMMAND, ENABLE]);
    let mut session =
        CosimSession::new_with_factory(&[config], &bus, |_| Ok(Box::new(Counter(calls.clone()))))
            .unwrap()
            .unwrap();
    session.advance_to(1, &mut bus).unwrap();
    let accepted_calls = calls.load(Ordering::SeqCst);
    assert!(accepted_calls > 0);
    let next_boundary = session.next_boundary_ns;
    state.lock().unwrap().driver = None;
    let error = session.advance_to(2, &mut bus).unwrap_err();
    assert!(error.to_string().contains(COMMAND), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), accepted_calls);
    assert_eq!(session.next_boundary_ns, next_boundary);
    assert!(!session.signals.contains_key(COMMAND));
}

#[test]
fn reset_hold_does_not_turn_an_unknown_driver_into_a_command() {
    let (mut bus, state) = bus(Some(PadDriver::OFF), External::Level(true));
    let router = router(&bus, false);
    let reset = serde_yaml::from_str(
        "pin: NRST\nactive_low: true\nvil_max_v: 0.8\nvih_min_v: 2.0\nmin_assert_ns: 0\ninvalid_pulse_ns: 0\n",
    ).unwrap();
    bus.reset_input = crate::reset_input::ResetInputState::new(Some(reset));
    bus.reset_input.attach();
    assert!(bus.reset_input.is_held());
    let mut signals = CosimSignals::new();
    for driver in [PadDriver::OFF, PadDriver::drive(true)] {
        state.lock().unwrap().driver = Some(driver);
        router.sample(&bus, &mut signals);
        assert_eq!(number(&signals, COMMAND), 0.0);
        assert_eq!(signals.get(ENABLE), Some(&CosimSignalValue::Bool(false)));
        router.require_drive_commands(&signals).unwrap();
    }
    state.lock().unwrap().driver = None;
    router.sample(&bus, &mut signals);
    assert!(!signals.contains_key(COMMAND));
    assert!(router.require_drive_commands(&signals).is_err());
    assert_eq!(state.lock().unwrap().external, External::Level(true));
}
