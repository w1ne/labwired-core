//! The four-state pin trace (`0`/`1`/`z`/`x`), on real firmware.
//!
//! `echo-capture.elf` (STM32F401, see `fixtures/timer-capture/`) drives PA1 as
//! a push-pull TRIG output and muxes PA0 to TIM2_CH1, where an HC-SR04 drives
//! ECHO. PA4 is never configured: a floating input nothing drives.
//!
//! The boolean trace of those three pads cannot tell "PA4 reads 0 because
//! nothing drives it" from "PA1 is driven low" — the four-state lane can, and
//! it must do so without changing a single boolean edge.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::logic_capture::{LogicSource, PadState};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Machine};
use std::path::PathBuf;

const GPIOA_IDR: u64 = 0x4002_0010;

fn machine() -> Machine<CortexM> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32f401.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: four-state\nchip: unused\nexternal_devices:\n  - id: range\n    type: hc-sr04\n    connection: gpio\n    config:\n      trig_pin: PA1\n      echo_pin: PA0\n      cpu_hz: 84000000\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(
        &root.join("crates/core/tests/fixtures/timer-capture/echo-capture.elf"),
    )
    .unwrap();
    m.load_firmware(&image).unwrap();
    m.bus.set_input(Some("range"), "distance", 10.0).unwrap();
    m
}

fn states_of(m: &mut Machine<CortexM>, ch: u32) -> Vec<(u64, PadState)> {
    m.logic_read_states(0)
        .edges
        .iter()
        .filter(|e| e.ch == ch)
        .map(|e| (e.cycle, e.state))
        .collect()
}

#[test]
fn undriven_input_is_z_driven_output_is_0_1_and_a_fight_is_x() {
    let mut m = machine();
    let gpioa = m.bus.find_peripheral_index_by_name("gpioa").unwrap();
    let watch = [
        Some(LogicSource::pad(gpioa, 0)), // ECHO on TIM2_CH1
        Some(LogicSource::pad(gpioa, 1)), // TRIG output
        Some(LogicSource::pad(gpioa, 4)), // never configured
    ];
    m.logic_watch(&watch);
    let initial = m.logic_initial_states().to_vec();
    // At reset every pad is an undriven input. The HC-SR04 model first puts
    // a level on ECHO at its first echo edge, so until then ECHO is z too.
    assert_eq!(initial, vec![Some(PadState::HighZ); 3]);

    m.advance(AdvanceRequest::run(Some(200_000))).unwrap();

    // TRIG: z → driven 0 (MODER output, ODR 0) → 1 → 0.
    let trig: Vec<PadState> = states_of(&mut m, 1).into_iter().map(|(_, s)| s).collect();
    assert_eq!(
        trig,
        vec![PadState::Low, PadState::High, PadState::Low],
        "TRIG four-state lane"
    );
    // ECHO: z until the sensor starts driving it (low, during the 200 µs
    // burst), then 1 and 0 — through the AF pad muxed to TIM2_CH1.
    let echo: Vec<PadState> = states_of(&mut m, 0).into_iter().map(|(_, s)| s).collect();
    assert_eq!(
        echo,
        vec![PadState::Low, PadState::High, PadState::Low],
        "ECHO four-state lane"
    );
    // PA4: nothing ever drove it; still z, no transitions.
    assert!(states_of(&mut m, 2).is_empty());

    // The boolean lane is untouched by all this: TRIG's pad level went
    // 0 → 1 → 0 (the z → 0 step is a DRIVE change and adds no edge).
    let bool_trig: Vec<bool> = m
        .logic_read_edges(0)
        .edges
        .iter()
        .filter(|e| e.ch == 1)
        .map(|e| e.value)
        .collect();
    assert_eq!(bool_trig, vec![true, false]);

    // Contention: an external driver holds TRIG high while the firmware
    // drives it low.
    let cursor = m.logic_read_states(0).cursor;
    assert!(m.bus.drive_input_bit(GPIOA_IDR, 1, true));
    m.advance(AdvanceRequest::run(Some(10))).unwrap();
    let fight: Vec<PadState> = m
        .logic_read_states(cursor)
        .edges
        .iter()
        .filter(|e| e.ch == 1)
        .map(|e| e.state)
        .collect();
    assert_eq!(fight, vec![PadState::Contention]);
}

#[test]
fn four_state_lane_serializes_as_extra_fields_on_the_series() {
    use labwired_core::logic_capture::{
        attach_logic_states, build_logic_edges_result, LogicChannelMeta,
    };
    let mut m = machine();
    let gpioa = m.bus.find_peripheral_index_by_name("gpioa").unwrap();
    let initial = m.logic_watch(&[Some(LogicSource::pad(gpioa, 1))]);
    m.advance(AdvanceRequest::run(Some(200_000))).unwrap();
    let meta = vec![LogicChannelMeta {
        ch: 0,
        peripheral: "gpioa".into(),
        pin: 1,
        initial: initial[0],
    }];
    let mut result = build_logic_edges_result(&meta, &m.logic_read_edges(0), 0);
    let bool_only = serde_json::to_value(&result).unwrap();
    assert!(bool_only["channels"][0].get("states").is_none());
    let states = m.logic_read_states(0);
    let init = m.logic_initial_states().to_vec();
    attach_logic_states(&mut result, &init, &states);
    let json = serde_json::to_value(&result).unwrap();
    let lane = &json["channels"][0];
    // Old fields keep their exact values.
    assert_eq!(lane["transitions"], bool_only["channels"][0]["transitions"]);
    assert_eq!(lane["initial_state"], "z");
    let seq: Vec<&str> = lane["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["state"].as_str().unwrap())
        .collect();
    assert_eq!(seq, vec!["0", "1", "0"]);
}
