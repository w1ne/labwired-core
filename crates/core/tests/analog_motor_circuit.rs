// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A brushed DC motor whose winding is an analog-netlist element
//! (`armature: circuit`), end to end on an ATmega328P.
//!
//! The firmware drives PB1 through a base resistor into an NPN low-side switch
//! with a flyback diode across the winding, the netlist's own 5 V rail
//! supplying it. The winding is series R + L + a back-EMF source; the netlist
//! solves its current, routes it into the motor on
//! `board.motor.M1.current_a`, and the motor routes its back-EMF back on
//! `board.motor.M1.back_emf_v`; the rail the winding hangs from comes in on
//! `board.motor.M1.supply_v` as the motor's bus voltage. The shaft carries
//! the catalog `dc_motor` friction (viscous plus dry Coulomb), so a coasting
//! fan stops rather than spinning down for seconds. Nothing here is motor-specific to the switch:
//! the same three elements sit under any driver a netlist can describe.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, RoutingError};
use labwired_core::cpu::avr::Avr;
use labwired_core::{AdvanceRequest, Bus, Machine};
use std::path::Path;

/// The motor and its low-side NPN driver, every motor figure the catalog
/// `dc_motor` default (configs/devices/dc_motor.yaml): 1.2 ohm, 2 mH,
/// Kt = Ke = 0.08, 40 g·cm² rotor, 10 µN·m·s/rad viscous and 6 mN·m Coulomb
/// friction. Base drive 1 k from PB1; 1N4001-style flyback from collector to
/// rail.
const MOTOR_SYSTEM: &str = r#"
motor_models:
  - kind: dc
    id: "M1"
    armature: circuit
    resistance_ohm: 1.2
    inductance_h: 0.002
    torque_constant_nm_per_a: 0.08
    back_emf_constant_v_per_rad_s: 0.08
    rotor_inertia_kg_m2: 0.00004
    viscous_friction_nm_per_rad_s: 0.00001
    coulomb_friction_nm: 0.006
    load_torque_nm: 0.0
    encoder_cpr: 12
    simulation_clock_hz: 16000000
cosim_models:
  - id: "circuit"
    adapter: "analog"
    step_ns: 100000
    inputs:
      drv_pb1: "board.gpio.pb1"
      ctl_pb1: "board.gpio_output.pb1"
      emf_M1: "board.motor.M1.back_emf_v"
    outputs:
      motor_M1: "board.motor.M1.current_a"
      supply_M1: "board.motor.M1.supply_v"
    config:
      vdd: 5.0
      netlist_text: |
        Vrail vcc 0 dc 5.0
        RM1 vcc mot_M1a 1.2
        LM1 mot_M1a mot_M1b 2m
        Vemf_M1 mot_M1b col dc 0
        QQ1 col base 0 QNPN
        RB1 drv base 1k
        DFLY col vcc DFLY
        Vdrv_pb1 d_pb1 0 dc 0
        Sdrv_pb1 d_pb1 drv ctl_pb1 ron=25 roff=1e12
        .model QNPN NPN (IS=1e-14 BF=200)
        .model DFLY D (IS=2.52e-9 N=1.752 RS=0)
      probes:
        motor_M1: "i(LM1)"
        supply_M1: "v(vcc)"
        collector: "v(col)"
      sources:
        drv_pb1: "Vdrv_pb1"
        emf_M1: "Vemf_M1"
"#;

/// `SBI DDRB,1 ; SBI PORTB,1 ; RJMP .` — fan on, then park.
const FAN_ON: [u16; 3] = [0x9A21, 0x9A29, 0xCFFF];
/// PORTB in the bus's AVR data window (PINB at 0x10023).
const PORTB: u64 = 0x0001_0025;
const CPU_HZ: u64 = 16_000_000;

fn manifest(system: &str) -> SystemManifest {
    let chip_path = root("configs/chips/atmega328p.yaml");
    serde_yaml::from_str(&format!(
        "name: \"motor-circuit\"\nchip: \"{}\"\nexternal_devices: []\n{system}",
        chip_path.display()
    ))
    .expect("the system parses")
}

struct Bench {
    machine: Machine<Avr>,
    session: CosimSession,
}

impl Bench {
    fn new() -> Self {
        Self::with_system(MOTOR_SYSTEM)
    }

    fn with_system(system: &str) -> Self {
        let manifest = manifest(system);
        let descriptor = ChipDescriptor::from_file(root("configs/chips/atmega328p.yaml"))
            .expect("load chip descriptor");
        let bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
        let mut cpu = Avr::new();
        cpu.load_words(0, &FAN_ON);
        let mut machine = Machine::new(cpu, bus);
        let session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
            .expect("build session")
            .expect("a model was declared");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "both motor paths bind to the circuit motor"
        );
        machine.attach_analog_trace(session.analog_trace_registry());
        Self { machine, session }
    }

    /// Run until `ms` of simulated time have passed since reset.
    fn run_to_ms(&mut self, ms: u64) {
        let target = ms * CPU_HZ / 1000;
        while self.machine.total_cycles < target {
            let advance = self
                .session
                .advance(&mut self.machine, AdvanceRequest::run(None))
                .expect("advance");
            assert!(
                advance.new_routing_errors.is_empty(),
                "{:?}",
                advance.new_routing_errors
            );
        }
    }

    fn motor(&self) -> labwired_core::bus::MotorSnapshot {
        self.machine
            .bus
            .motor_snapshots()
            .into_iter()
            .find(|m| m.id == "M1")
            .expect("motor M1")
    }

    fn fan_off(&mut self) {
        self.machine.bus.write_u8(PORTB, 0).expect("clear PORTB");
    }
}

/// `(time_ns, value)` of the series point holding at `ms`.
fn held_at(points: &[(u64, f64)], ms: u64) -> f64 {
    points
        .iter()
        .take_while(|p| p.0 <= ms * 1_000_000)
        .last()
        .map(|p| p.1)
        .unwrap_or(0.0)
}

#[test]
fn switched_on_the_winding_carries_current_and_the_shaft_spins() {
    let mut bench = Bench::new();
    bench.run_to_ms(400);
    let motor = bench.motor();
    assert_eq!(motor.control_state, "circuit");
    assert!(motor.speed_rpm > 100.0, "spinning: {} rpm", motor.speed_rpm);
    let current = motor.current_a.expect("a routed winding current");
    assert!(current > 0.0, "forward winding current: {current} A");
    assert!(
        (motor.bus_voltage_v - 5.0).abs() < 0.01,
        "the motor reports the netlist rail it hangs from: {} V",
        motor.bus_voltage_v
    );
}

#[test]
fn back_emf_pulls_the_running_current_below_the_inrush() {
    // A stiffer base drive so the switch, not the transistor's gain, sets
    // the inrush: what is left to limit the running current is back-EMF.
    let mut bench =
        Bench::with_system(&MOTOR_SYSTEM.replace("RB1 drv base 1k", "RB1 drv base 220"));
    bench.run_to_ms(1500);
    let series = bench.machine.analog_series().expect("a series");
    let current = series
        .channels
        .iter()
        .find(|c| c.name == "motor.M1.current_a")
        .expect("motor current channel");
    assert_eq!(current.unit, "A");
    let peak = current.points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let running = bench.motor().current_a.expect("current");
    assert!(
        running < 0.5 * peak,
        "running current {running} A should sit well below the {peak} A inrush"
    );
    // The netlist's own probe agrees with what the motor was handed.
    let probe = series
        .channels
        .iter()
        .find(|c| c.name == "circuit.motor_M1")
        .expect("netlist winding probe");
    let probed = probe.points.last().expect("points").1;
    assert!(
        (probed - running).abs() <= 0.05 * peak,
        "probe {probed} A vs motor {running} A"
    );
}

#[test]
fn switched_off_the_winding_current_decays_and_the_shaft_stops() {
    let mut bench = Bench::new();
    bench.run_to_ms(400);
    let on_current = bench.motor().current_a.expect("current");
    let on_speed = bench.motor().speed_rpm;
    bench.fan_off();
    bench.run_to_ms(1400);
    let series = bench.machine.analog_series().expect("a series");
    let collector = series
        .channels
        .iter()
        .find(|c| c.name == "circuit.collector")
        .expect("collector channel");
    let peak = collector
        .points
        .iter()
        .filter(|p| p.0 > 400_000_000)
        .map(|p| p.1)
        .fold(f64::MIN, f64::max);
    assert!(
        peak < 5.0 + 1.5,
        "the diode clamps the collector at most one drop above the rail: peak {peak} V"
    );
    let off = bench.motor();
    assert!(
        off.current_a.expect("current").abs() < 0.1 * on_current.abs().max(1e-3),
        "winding current decayed: {:?} A (was {on_current} A)",
        off.current_a
    );
    assert!(
        on_speed > 100.0,
        "spinning before the switch opened: {on_speed} rpm"
    );
    assert_eq!(
        off.speed_rpm, 0.0,
        "dry friction stops the fan within a second of the switch opening"
    );
    // And it stays stopped: stiction holds a shaft nothing drives.
    bench.run_to_ms(1900);
    assert_eq!(bench.motor().speed_rpm, 0.0);
}

/// The catalog fan's 83 mA no-load current freewheels out through the diode
/// in tens of microseconds, under one netlist sample. A stalled rotor (dry
/// friction past the stall torque) has no back-EMF and carries the full
/// inrush, so its switch-off shows the clamp itself.
#[test]
fn a_stalled_winding_switched_off_freewheels_through_the_flyback_diode() {
    let mut bench = Bench::with_system(
        &MOTOR_SYSTEM.replace("coulomb_friction_nm: 0.006", "coulomb_friction_nm: 10.0"),
    );
    bench.run_to_ms(100);
    assert_eq!(bench.motor().speed_rpm, 0.0, "the rotor is held");
    let on_current = bench.motor().current_a.expect("current");
    // The 1 k base drive limits the NPN to well under the 4 A the winding alone would draw.
    assert!(on_current > 0.5, "stall current: {on_current} A");
    bench.fan_off();
    bench.run_to_ms(200);
    let series = bench.machine.analog_series().expect("a series");
    let collector = series
        .channels
        .iter()
        .find(|c| c.name == "circuit.collector")
        .expect("collector channel");
    let peak = collector
        .points
        .iter()
        .filter(|p| p.0 > 100_000_000)
        .map(|p| p.1)
        .fold(f64::MIN, f64::max);
    assert!(
        peak > 5.0 && peak < 5.0 + 1.5,
        "the diode clamps the collector one drop above the rail: peak {peak} V"
    );
    let off = bench.motor().current_a.expect("current");
    assert!(
        off.abs() < 0.01 * on_current,
        "winding current decayed: {off} A (was {on_current} A)"
    );
}

#[test]
fn the_speed_series_covers_the_run_in_change_points() {
    let mut bench = Bench::new();
    bench.run_to_ms(300);
    bench.fan_off();
    bench.run_to_ms(1400);
    let series = bench.machine.analog_series().expect("a series");
    assert!(
        series.end_time_ns >= 1_390_000_000,
        "{}",
        series.end_time_ns
    );
    let speed = series
        .channels
        .iter()
        .find(|c| c.name == "motor.M1.speed_rpm")
        .expect("motor speed channel");
    assert_eq!(speed.unit, "rpm");
    assert_eq!(speed.dropped, 0);
    for pair in speed.points.windows(2) {
        assert!(pair[0].0 < pair[1].0, "points are time-ordered");
    }
    let rising = held_at(&speed.points, 300);
    assert!(rising > 100.0, "spinning at 300 ms: {rising} rpm");
    let coasting = held_at(&speed.points, 500);
    assert!(
        coasting > 0.0 && coasting < rising,
        "spinning down 200 ms after the switch opened: {coasting} rpm"
    );
    assert_eq!(
        held_at(&speed.points, 1300),
        0.0,
        "stopped within a second of the switch opening"
    );
    // Change points, not one per motor service.
    let services = 1_400_000_000 / (4096 * 1_000_000_000 / CPU_HZ);
    assert!(
        (speed.points.len() as u64) < services,
        "{} points",
        speed.points.len()
    );
}

#[test]
fn a_bridge_motor_refuses_the_circuit_paths() {
    let bridge = MOTOR_SYSTEM
        .replace("    armature: circuit\n", "    armature: bridge\n    pwm_pin: \"PB1\"\n    direction_pin: \"3V3\"\n    supply_voltage_v: 5.0\n");
    let bridge_manifest = manifest(&bridge);
    let descriptor =
        ChipDescriptor::from_file(root("configs/chips/atmega328p.yaml")).expect("descriptor");
    let bus = SystemBus::from_config(&descriptor, &bridge_manifest).expect("build bus");
    let session = CosimSession::new(&bridge_manifest.cosim_models, Path::new("."), &bus)
        .expect("build session")
        .expect("a model");
    let errors = session.binding_errors();
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, RoutingError::NotCircuitMotor { motor, .. } if motor == "M1")),
        "{errors:?}"
    );
    let unknown = MOTOR_SYSTEM.replace("board.motor.M1.", "board.motor.M9.");
    let unknown_manifest = manifest(&unknown);
    let bus = SystemBus::from_config(&descriptor, &unknown_manifest).expect("build bus");
    let session = CosimSession::new(&unknown_manifest.cosim_models, Path::new("."), &bus)
        .expect("build session")
        .expect("a model");
    assert!(session
        .binding_errors()
        .iter()
        .any(|e| matches!(e, RoutingError::UnknownMotor { motor, .. } if motor == "M9")));
}

#[test]
fn no_routed_current_holds_the_shaft_instead_of_guessing() {
    let no_output = MOTOR_SYSTEM.replace(
        "    outputs:\n      motor_M1: \"board.motor.M1.current_a\"\n      supply_M1: \"board.motor.M1.supply_v\"\n",
        "    outputs: {}\n",
    );
    let manifest = manifest(&no_output);
    let descriptor =
        ChipDescriptor::from_file(root("configs/chips/atmega328p.yaml")).expect("descriptor");
    let bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
    let mut cpu = Avr::new();
    cpu.load_words(0, &FAN_ON);
    let mut machine = Machine::new(cpu, bus);
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
        .expect("session")
        .expect("a model");
    while machine.total_cycles < 100 * CPU_HZ / 1000 {
        session
            .advance(&mut machine, AdvanceRequest::run(None))
            .expect("advance");
    }
    let motor = machine
        .bus
        .motor_snapshots()
        .into_iter()
        .find(|m| m.id == "M1")
        .unwrap();
    assert_eq!(motor.control_state, "circuit:no-current");
    assert_eq!(motor.speed_rpm, 0.0);
    assert_eq!(motor.current_a, None);
    assert_eq!(motor.bus_voltage_v, 0.0, "no rail routed, none reported");
}
