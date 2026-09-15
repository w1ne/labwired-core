// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `CosimSession::advance` and `CosimSession::advance_budget`: the one place a
//! machine is stepped in lockstep with its co-simulation models.
//!
//! `labwired test` issues one lockstep advance per loop iteration and the
//! browser engine spends a whole `step_batch` budget per call; both go through
//! these two methods, so the lockstep rule is asserted here once against a real
//! machine running real firmware.
//!
//! The firmware is the committed NUCLEO-F401RE Arduino blink image. At 84 MHz a
//! 100 us model period is 8400 cycles.

use labwired_config::{ChipDescriptor, CosimAdapter, CosimModelConfig, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::routing::cycles_to_ns;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, AdvanceStop, Bus, Machine};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const STEP_NS: u64 = 100_000;
const BOUNDARY_CYCLES: u64 = 8_400;

/// Cycles an atomic machine boundary may charge past a cycle budget (see
/// `Machine::advance`). Generous: the point is "on the boundary, not a period
/// past it".
const BOUNDARY_SLACK: u64 = 64;

fn root(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn f401_machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/stm32f401.yaml");
    let descriptor = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"cosim-advance\"\nchip: \"{}\"\nexternal_devices: []\n",
        chip_path.display()
    ))
    .expect("parse manifest");
    let mut bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&root("tests/fixtures/stm32f401-blinky.elf"))
        .expect("load blinky fixture");
    machine.load_firmware(&image).expect("load firmware");
    machine
}

fn mock_model(
    outputs: &[(&str, &str)],
    static_outputs: &[(&str, serde_yaml::Value)],
) -> CosimModelConfig {
    let mapping: serde_yaml::Mapping = static_outputs
        .iter()
        .map(|(k, v)| (serde_yaml::Value::String((*k).to_string()), v.clone()))
        .collect();
    CosimModelConfig {
        id: "probe".to_string(),
        adapter: CosimAdapter::Mock,
        model: None,
        step_ns: STEP_NS,
        inputs: HashMap::from([("led".to_string(), "board.gpio.pa5".to_string())]),
        outputs: outputs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        config: HashMap::from([("outputs".to_string(), serde_yaml::Value::Mapping(mapping))]),
    }
}

fn session_for(machine: &Machine<CortexM>, models: &[CosimModelConfig]) -> CosimSession {
    let session = CosimSession::new(models, Path::new("."), &machine.bus)
        .expect("build session")
        .expect("models were declared");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    session
}

fn contact_model() -> CosimModelConfig {
    mock_model(
        &[("contact", "board.gpio_in.pc13")],
        &[("contact", serde_yaml::Value::Bool(true))],
    )
}

fn pc13_idr(machine: &mut Machine<CortexM>) -> bool {
    let (addr, bit) =
        SystemBus::resolve_pin_idr_pub(&machine.bus, "pc13").expect("PC13 resolves to an IDR");
    machine.bus.read_u32(addr).expect("read IDR") >> bit & 1 != 0
}

/// One lockstep advance runs the machine TO the next boundary, not past it,
/// and steps the model there — even for a request with no budget of its own.
#[test]
fn one_advance_lands_on_the_boundary_and_steps_the_model_there() {
    let mut machine = f401_machine();
    let mut session = session_for(&machine, &[contact_model()]);
    assert!(
        !pc13_idr(&mut machine),
        "PC13 idles low before any model step"
    );

    let advance = session
        .advance(&mut machine, AdvanceRequest::run(None))
        .expect("advance");

    assert_eq!(advance.report.stop, AdvanceStop::CycleLimit);
    assert!(
        (BOUNDARY_CYCLES..BOUNDARY_CYCLES + BOUNDARY_SLACK).contains(&machine.total_cycles),
        "the machine should stop on the 8400-cycle boundary, stopped at {}",
        machine.total_cycles
    );
    assert_eq!(advance.report.elapsed_cycles, machine.total_cycles);
    assert_eq!(
        advance.routed.len(),
        1,
        "the model steps once, at the boundary"
    );
    assert!(advance.new_routing_errors.is_empty());
    assert!(pc13_idr(&mut machine), "the routed output reached PC13");
}

/// A request's own cycle budget tighter than the boundary wins, and the model
/// is not stepped before its boundary.
#[test]
fn a_budget_short_of_the_boundary_steps_no_model() {
    let mut machine = f401_machine();
    let mut session = session_for(&machine, &[contact_model()]);

    let advance = session
        .advance(
            &mut machine,
            AdvanceRequest::run(None).with_cycle_limit(1_000),
        )
        .expect("advance");

    assert_eq!(advance.report.stop, AdvanceStop::CycleLimit);
    assert!(machine.total_cycles < BOUNDARY_CYCLES);
    assert!(advance.routed.is_empty());
    assert!(!pc13_idr(&mut machine));
}

/// A fuel budget spanning many periods is spent in full, and every boundary on
/// the way is stepped exactly once.
#[test]
fn a_fuel_budget_spans_boundaries_and_steps_each_one() {
    const FUEL: u64 = 100_000;
    let mut machine = f401_machine();
    let mut session = session_for(&machine, &[contact_model()]);

    let advance = session
        .advance_budget(&mut machine, AdvanceRequest::run(Some(FUEL)))
        .expect("advance");

    assert_eq!(advance.report.stop, AdvanceStop::FuelLimit);
    assert_eq!(
        advance.report.fuel_consumed, FUEL,
        "the whole budget is spent across boundaries"
    );
    assert_eq!(advance.report.elapsed_cycles, machine.total_cycles);
    let boundaries = cycles_to_ns(machine.total_cycles, 84_000_000) / STEP_NS;
    assert!(boundaries > 2, "the budget must cross several periods");
    assert_eq!(advance.routed.len() as u64, boundaries);
}

/// A cycle budget spanning periods: stops at the budget, with each boundary
/// before it stepped.
#[test]
fn a_cycle_budget_spans_boundaries() {
    let mut machine = f401_machine();
    let mut session = session_for(&machine, &[contact_model()]);

    let advance = session
        .advance_budget(
            &mut machine,
            AdvanceRequest::run(None).with_cycle_limit(20_000),
        )
        .expect("advance");

    assert_eq!(advance.report.stop, AdvanceStop::CycleLimit);
    assert!((20_000..20_000 + BOUNDARY_SLACK).contains(&machine.total_cycles));
    assert_eq!(advance.routed.len(), 2, "boundaries at 8400 and 16800");
}

/// A single-step request steps one quantum; a model is stepped only when that
/// quantum reached its boundary.
#[test]
fn a_single_step_steps_one_quantum() {
    let mut machine = f401_machine();
    let mut session = session_for(&machine, &[contact_model()]);

    let advance = session
        .advance_budget(&mut machine, AdvanceRequest::single())
        .expect("advance");

    assert_eq!(advance.report.primary_steps, 1);
    assert!(advance.routed.is_empty());
}

/// A route that fails at apply time (a text value on an analog pad) is
/// reported by the first advance that hits it, and not again: `labwired test`
/// logs each distinct failure once, and a browser console must not get one
/// line per 100 us either.
#[test]
fn a_runtime_routing_failure_is_reported_once() {
    let mut machine = f401_machine();
    let model = mock_model(
        &[("volts", "board.analog.pa0_volts")],
        &[("volts", serde_yaml::Value::from("high"))],
    );
    let mut session = session_for(&machine, &[model]);

    let first = session
        .advance(&mut machine, AdvanceRequest::run(None))
        .expect("advance");
    assert_eq!(
        first.new_routing_errors,
        vec![RoutingError::TypeMismatch {
            path: "board.analog.pa0_volts".to_string(),
            value: CosimSignalValue::Text("high".to_string()),
        }]
    );
    let second = session
        .advance(&mut machine, AdvanceRequest::run(None))
        .expect("advance");
    assert_eq!(second.routed.len(), 1, "the model still steps");
    assert!(second.new_routing_errors.is_empty());
}
