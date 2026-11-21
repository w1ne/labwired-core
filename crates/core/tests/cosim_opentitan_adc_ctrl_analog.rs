// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the OpenTitan Earl
//! Grey ADC controller (`opentitan_adc_ctrl` at 0x4044_0000, `adc_ctrl_aon`
//! in top_earlgrey.gen.hjson) on the onboarding `opentitan-earlgrey` and
//! `opentitan-earlgrey-cw310` profiles through the descriptor's
//! `analog_pins` map. CC1 is AST channel 0 and CC2 channel 1
//! (chip_earlgrey_asic.sv). The NTC is on CC1 and the diode on CC2, read
//! two ways:
//! - oneshot, as `dif_adc_ctrl`: `adc_pd_ctl.pwrup_time`, `adc_intr_ctl`
//!   `ONESHOT_EN`, `adc_en_ctl = oneshot_mode | adc_enable`, wait for
//!   `adc_intr_status.oneshot`, read `adc_chn_val[n]` [11:2] and the latched
//!   [27:18], clear the status, then `dif_adc_ctrl_reset` (`adc_fsm_rst` 1
//!   then 0) to take the next one;
//! - as a debounced threshold, the debug-cable use the IP is built for:
//!   low-power scanning with a filter pair (CC1 below the 40 °C divider
//!   code AND CC2 inside the diode window); `lp_sample_cnt` hits move the
//!   FSM to normal power (`trans`), `np_sample_cnt` hits raise `match`, and
//!   `adc_chn_value_intr` holds the codes of that first debounced hit.
//!
//! 10 bits, `code = mV * 1023 / 2300` (Earl Grey's Nuvoton integration in
//! the adc_ctrl programmer guide), so the -10 °C divider (2.79 V) saturates at
//! 0x3FF and the 25 °C one (1.65 V) does not.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC_CTRL: u64 = 0x4044_0000;
const INTR_STATE: u64 = ADC_CTRL;
const INTR_ENABLE: u64 = ADC_CTRL + 0x04;
const ADC_EN_CTL: u64 = ADC_CTRL + 0x10;
const ADC_PD_CTL: u64 = ADC_CTRL + 0x14;
const ADC_LP_SAMPLE_CTL: u64 = ADC_CTRL + 0x18;
const ADC_SAMPLE_CTL: u64 = ADC_CTRL + 0x1C;
const ADC_FSM_RST: u64 = ADC_CTRL + 0x20;
const ADC_CHN0_FILTER_CTL: u64 = ADC_CTRL + 0x24;
const ADC_CHN1_FILTER_CTL: u64 = ADC_CTRL + 0x44;
const ADC_CHN_VAL: u64 = ADC_CTRL + 0x64;
const FILTER_STATUS: u64 = ADC_CTRL + 0x70;
const ADC_INTR_CTL: u64 = ADC_CTRL + 0x74;
const ADC_INTR_STATUS: u64 = ADC_CTRL + 0x78;
const ADC_FSM_STATE: u64 = ADC_CTRL + 0x7C;
const ADC_ENABLE: u32 = 1;
const ONESHOT_MODE: u32 = 1 << 1;
const LP_MODE: u32 = 1;
const TRANS: u32 = 1 << 8;
const ONESHOT: u32 = 1 << 9;
const FILTER_EN: u32 = 1 << 31;
const NP_0: u32 = 12;
const VREF_MV: u32 = 2300;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    3.3 * r / (10_000.0 + r)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// 10-bit, 1023 / VREF, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 1023 / VREF_MV).min(1023)
}

/// An enabled filter, `min <= v <= max`.
fn inside(min: u32, max: u32) -> u32 {
    FILTER_EN | (max << 18) | (min << 2)
}

/// The onboarding profile with the generic NTC and diode netlists routed to
/// the two pads.
fn board(profile_name: &str, ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root(&format!("configs/systems/onboarding/{profile_name}.yaml"));
    let mut profile = SystemManifest::from_file(&system_path).expect("load Earl Grey profile");
    let analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_pad),
            "diode_load_line" => ("v_diode", diode_pad),
            other => panic!("unexpected model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load Earl Grey descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build Earl Grey bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to AST channels through analog_pins"
    );
    (bus, session)
}

/// Move the NTC and solve both netlists to the next boundary.
fn settle(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64, temperature_c: f64) {
    session
        .set_signal_number("ui.ntc.temperature_c", temperature_c)
        .expect("set NTC temperature");
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route both netlists");
    assert_eq!(steps.len(), 2, "both netlists solve each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
}

fn adc_irq(bus: &SystemBus) -> bool {
    let adc = bus
        .peripherals
        .iter()
        .find(|p| p.name == "adc_ctrl")
        .expect("adc_ctrl on the bus");
    adc.dev
        .irq_line_level()
        .expect("opentitan_adc_ctrl reports its request")
}

/// Tick until `adc_intr_status & bits`; the tick count.
fn wait_status(bus: &mut SystemBus, bits: u32, limit: usize) -> usize {
    for n in 1..=limit {
        bus.tick_peripherals();
        if bus.read_u32(ADC_INTR_STATUS).unwrap() & bits != 0 {
            return n;
        }
    }
    panic!("adc_intr_status {bits:#x} not set in {limit} ticks");
}

/// `(latest, latched)` from `adc_chn_val[channel]`.
fn chn_val(bus: &mut SystemBus, channel: u64) -> (u32, u32) {
    let reg = bus.read_u32(ADC_CHN_VAL + 4 * channel).unwrap();
    ((reg >> 2) & 0x3FF, (reg >> 18) & 0x3FF)
}

/// One `dif_adc_ctrl` oneshot: both channels' codes.
fn oneshot(bus: &mut SystemBus) -> [u32; 2] {
    bus.write_u32(ADC_FSM_RST, 1).expect("dif_adc_ctrl_reset");
    bus.write_u32(ADC_FSM_RST, 0).expect("dif_adc_ctrl_reset");
    bus.write_u32(ADC_EN_CTL, ONESHOT_MODE | ADC_ENABLE)
        .expect("oneshot + enable");
    wait_status(bus, ONESHOT, 64);
    assert_eq!(bus.read_u32(INTR_STATE).unwrap(), 1, "status interrupt");
    let codes = [0, 1].map(|ch| {
        let (latest, latched) = chn_val(bus, ch);
        assert_eq!(latest, latched, "oneshot_done latches channel {ch}");
        latest
    });
    bus.write_u32(ADC_INTR_STATUS, ONESHOT)
        .expect("clear oneshot");
    codes
}

fn open_oneshot(bus: &mut SystemBus) {
    // pwrup_time 7 (reset), wakeup_time 1600, normal power.
    bus.write_u32(ADC_PD_CTL, (1600 << 8) | (7 << 4))
        .expect("adc_pd_ctl");
    bus.write_u32(ADC_INTR_CTL, ONESHOT).expect("ONESHOT_EN");
    bus.write_u32(INTR_ENABLE, 1).expect("match_pending enable");
}

#[test]
fn earlgrey_cc1_and_cc2_convert_ntc_and_diode_by_oneshot() {
    let (mut bus, mut session) = board("opentitan-earlgrey", "CC1", "CC2");
    assert_eq!(bus.read_u32(ADC_FSM_STATE).unwrap(), 0, "PWRDN at reset");
    bus.tick_peripherals();
    assert_eq!(bus.read_u32(ADC_CHN_VAL).unwrap(), 0, "no sample while off");
    open_oneshot(&mut bus);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [25.0_f64, 50.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.CC1_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.CC2_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let [ntc_code, diode_code] = oneshot(&mut bus);
        assert_eq!(ntc_code, code(ntc_v), "CC1 at {temperature_c} °C");
        assert!(ntc_code < 1023, "below full scale at {temperature_c} °C");
        assert_eq!(diode_code, code(diode_v), "CC2");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // 2.15 mV per step: the 25 °C reading is within one step of the divider.
    let mv = ntc_codes[0] * VREF_MV / 1023;
    let true_mv = millivolts(ntc_volts(25.0));
    assert!(
        mv <= true_mv && true_mv - mv <= 3,
        "{mv} mV vs {true_mv} mV"
    );
    // At -10 °C the divider (about 2.8 V) is above the 2.3 V full scale.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    assert_eq!(oneshot(&mut bus)[0], 1023, "over-range clamps");
    assert!(!adc_irq(&bus), "status cleared, line low");
}

#[test]
fn earlgrey_lp_scan_debounces_the_warm_ntc_and_latches_its_code() {
    let (mut bus, mut session) = board("opentitan-earlgrey", "CC1", "CC2");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let diode = code(signal_voltage(&session, "board.analog.CC2_volts"));
    // Filter pair 3: CC1 at or below the 40 °C code AND CC2 within 10
    // codes of the diode. Only pair 3 is enabled for interrupts.
    let threshold = code(ntc_volts(40.0));
    bus.write_u32(ADC_CHN0_FILTER_CTL + 12, inside(0, threshold))
        .expect("chn0 filter 3");
    bus.write_u32(ADC_CHN1_FILTER_CTL + 12, inside(diode - 10, diode + 10))
        .expect("chn1 filter 3");
    bus.write_u32(ADC_LP_SAMPLE_CTL, 2).expect("lp_sample_cnt");
    bus.write_u32(ADC_SAMPLE_CTL, 3).expect("np_sample_cnt");
    bus.write_u32(ADC_INTR_CTL, TRANS | (1 << 3))
        .expect("TRANS_EN | MATCH_EN[3]");
    bus.write_u32(INTR_ENABLE, 1).expect("intr enable");
    // lp_mode, pwrup_time 2, wakeup_time 20 aon ticks.
    bus.write_u32(ADC_PD_CTL, (20 << 8) | (2 << 4) | LP_MODE)
        .expect("adc_pd_ctl");
    bus.write_u32(ADC_EN_CTL, ADC_ENABLE).expect("enable");

    // 25 °C: 1.65 V is above the 40 °C threshold; the FSM keeps sleeping.
    for _ in 0..400 {
        bus.tick_peripherals();
    }
    assert_eq!(bus.read_u32(FILTER_STATUS).unwrap(), 0, "no hit at 25 °C");
    assert!(!adc_irq(&bus));
    let (latest, _) = chn_val(&mut bus, 0);
    assert_eq!(latest, code(ntc_volts(25.0)), "periodic samples continue");

    // 50 °C: two LP hits move the FSM to normal power and flag `trans`.
    settle(&mut session, &mut bus, &mut cycles, 50.0);
    let warm = code(signal_voltage(&session, "board.analog.CC1_volts"));
    assert!(warm <= threshold);
    wait_status(&mut bus, TRANS, 200);
    assert!(adc_irq(&bus), "trans raises the status interrupt");
    assert_eq!(bus.read_u32(ADC_FSM_STATE).unwrap(), NP_0);
    assert_eq!(bus.read_u32(FILTER_STATUS).unwrap(), TRANS);
    bus.write_u32(ADC_INTR_STATUS, TRANS).expect("clear trans");
    assert!(!adc_irq(&bus));
    // Three NP hits debounce it: `match[3]` and the latched codes.
    let ticks = wait_status(&mut bus, 1 << 3, 64);
    assert_eq!(ticks, 3 * 3 + 1, "three NP pairs, then NP_DONE");
    assert_eq!(bus.read_u32(FILTER_STATUS).unwrap(), TRANS | (1 << 3));
    assert_eq!(chn_val(&mut bus, 0).1, warm, "CC1 latched at the hit");
    assert_eq!(chn_val(&mut bus, 1).1, diode, "CC2 latched at the hit");
    bus.write_u32(ADC_INTR_STATUS, !0).expect("clear");

    // 80 °C: still inside the same filter set; no second interrupt, the
    // latch keeps the first hit while the live value moves.
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    for _ in 0..100 {
        bus.tick_peripherals();
    }
    assert!(!adc_irq(&bus), "a stable match does not repeat");
    assert_eq!(chn_val(&mut bus, 0), (code(ntc_volts(80.0)), warm));

    // Cold again: the match drops and the FSM returns to LP scanning.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    // At most one stale evaluation, then a fresh pair and NP_EVAL.
    for _ in 0..7 {
        bus.tick_peripherals();
    }
    let state = bus.read_u32(ADC_FSM_STATE).unwrap();
    assert!((6..=11).contains(&state), "LP state, got {state}");
    for _ in 0..400 {
        bus.tick_peripherals();
    }
    assert_eq!(bus.read_u32(ADC_INTR_STATUS).unwrap(), 0, "no hit cold");
}

#[test]
fn earlgrey_cw310_cc2_and_cc1_convert_after_the_netlists_swap() {
    let (mut bus, mut session) = board("opentitan-earlgrey-cw310", "CC2", "CC1");
    open_oneshot(&mut bus);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.CC2_volts");
    let diode_v = signal_voltage(&session, "board.analog.CC1_volts");
    let [cc1, cc2] = oneshot(&mut bus);
    assert_eq!(cc2, code(ntc_v), "CC2");
    assert_eq!(cc1, code(diode_v), "CC1");
    assert_ne!(code(ntc_v), code(diode_v), "the swap is observable");
}
