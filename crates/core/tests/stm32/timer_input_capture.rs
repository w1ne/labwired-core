//! STM32 general-purpose timer INPUT CAPTURE, exercised by real firmware.
//!
//! The fixtures in `tests/fixtures/timer-capture/` are register-level
//! STM32F401 programs that configure TIM2 exactly the way a CubeMX project
//! does and record what they MEASURE in SRAM. They hold no expected values.
//! These tests inject known pulses on PA0 (`TIM2_CH1`, AF1) and compare.
//!
//! * `echo-capture.elf` — HC-SR04 ranging: CH1 latches the rising edge, CH2
//!   (TI1, falling) the falling edge, and the CC2 interrupt computes the
//!   echo width in microseconds. The pulse comes from the HC-SR04 device model
//!   through the same pad seam the browser uses.
//! * `freq-meter.elf` — PWM-input mode (slave reset on TI1FP1): CCR1 is the
//!   period, CCR2 the high time. The test drives the pad itself and records
//!   the exact engine cycle of every edge it drove, so the assertion is
//!   equality, not a tolerance.
//!
//! The negative controls route the same signal to a pad that is NOT a timer
//! input, and must measure nothing.
//!
//! Deliberately NOT behind `#![cfg(feature = "event-scheduler")]`: the same
//! assertions hold on the legacy walk (default features) and on the
//! scheduler path the browser and CLI ship (`--features event-scheduler`),
//! and both builds run this file.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Machine};
use std::path::PathBuf;

const RESULT: u64 = 0x2000_0100;
/// GPIOA input data register (RM0368 §8.4.5).
const GPIOA_IDR: u64 = 0x4002_0010;
const F401_HZ: f64 = 84_000_000.0;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn machine(external_devices: &str, firmware: &str) -> Machine<CortexM> {
    let chip = ChipDescriptor::from_file(root().join("configs/chips/stm32f401.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: timer-capture\nchip: unused\nexternal_devices:{external_devices}\n"
    ))
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(
        &root()
            .join("crates/core/tests/fixtures/timer-capture")
            .join(firmware),
    )
    .unwrap();
    machine.load_firmware(&image).unwrap();
    machine
}

fn results(m: &Machine<CortexM>, n: u64) -> Vec<u32> {
    (0..n)
        .map(|i| m.bus.read_u32(RESULT + i * 4).unwrap())
        .collect()
}

fn hc_sr04(echo_pin: &str) -> String {
    format!(
        "\n  - id: range\n    type: hc-sr04\n    connection: gpio\n    config:\n      trig_pin: PA1\n      echo_pin: {echo_pin}\n      cpu_hz: 84000000\n"
    )
}

/// Run the echo firmware with the sensor at `distance_cm`; return
/// `[phase, rise, fall, width_us, isr_count, sr_in_isr]`.
fn echo(distance_cm: f64, echo_pin: &str) -> Vec<u32> {
    let mut m = machine(&hc_sr04(echo_pin), "echo-capture.elf");
    m.bus
        .set_input(Some("range"), "distance", distance_cm)
        .unwrap();
    // 400 cm is 23.2 ms of echo = 1.95 M cycles at 84 MHz; the firmware
    // sleeps in WFI between edges, so this is cheap.
    m.advance(AdvanceRequest::run(Some(3_000_000))).unwrap();
    results(&m, 6)
}

#[test]
fn hc_sr04_echo_width_is_measured_by_tim2_input_capture() {
    for distance in [2.0, 24.25, 97.0, 172.4, 400.0] {
        let r = echo(distance, "PA0");
        assert_eq!(r[0], 2, "{distance} cm: the CC2 interrupt must have run");
        // The echo is `distance × 58 µs` long (HC-SR04 datasheet). TIM2 counts
        // one tick per µs (PSC=83 at 84 MHz), and the two edges fall at
        // arbitrary phases of that tick, so the tick count between them is
        // the floor or the ceiling of the true width — never further off.
        let expected_us = distance * 58.0;
        assert!(
            (f64::from(r[3]) - expected_us).abs() <= 1.0,
            "{distance} cm: measured {} µs, injected {expected_us} µs (rise {} fall {})",
            r[3],
            r[1],
            r[2]
        );
        assert!(r[2] > r[1], "capture order: rise before fall");
        // The echo starts 200 µs after TRIG (8 cycles of 40 kHz burst); the
        // rising capture lands there, not at the firmware's next read.
        assert!(r[1] >= 200, "{distance} cm: rise captured at {} µs", r[1]);
        // One falling edge, one CC2 interrupt: reading CCR2 in the ISR clears
        // CC2IF, which drops the level, so the ISR does not re-enter.
        assert_eq!(r[4], 1, "{distance} cm: exactly one TIM2 interrupt");
        assert_ne!(
            r[5] & (1 << 2),
            0,
            "{distance} cm: CC2IF was set in the ISR"
        );
        assert_eq!(r[5] & (1 << 10), 0, "{distance} cm: no CC2 over-capture");
    }
}

#[test]
fn hc_sr04_beyond_rated_range_answers_with_the_38_ms_timeout_pulse() {
    // Datasheet: no target within 400 cm -> ECHO high ~38 ms, never a range.
    for distance in [400.5, 401.0, 650.0, 1000.0] {
        let mut m = machine(&hc_sr04("PA0"), "echo-capture.elf");
        m.bus
            .set_input(Some("range"), "distance", distance)
            .unwrap();
        m.advance(AdvanceRequest::run(Some(3_600_000))).unwrap();
        let r = results(&m, 6);
        assert_eq!(r[0], 2, "{distance} cm: the CC2 interrupt must have run");
        assert!(
            (i64::from(r[3]) - 38_000).abs() <= 1,
            "{distance} cm: echo {} us, want 38000",
            r[3]
        );
    }
    // 400 cm itself is still a proportional echo.
    let r = echo(400.0, "PA0");
    assert!(
        (f64::from(r[3]) - 23_200.0).abs() <= 1.0,
        "400 cm: {} us",
        r[3]
    );
}

#[test]
fn echo_on_a_pad_that_is_not_a_timer_input_is_never_captured() {
    // Negative control: identical firmware and sensor, ECHO wired to PA4,
    // which has no TIM2 function. The firmware triggers (phase 1) and never
    // sees a capture.
    let r = echo(97.0, "PA4");
    assert_eq!(r[0], 1, "triggered, but no capture interrupt: {r:?}");
    assert_eq!(r[3], 0);
    assert_eq!(r[4], 0);
}

/// Step the machine until at least `target` engine cycles have elapsed, then
/// drive PA0 to `level` and return the exact cycle the edge was delivered at.
fn edge_at(m: &mut Machine<CortexM>, target: u64, level: bool) -> u64 {
    while m.total_cycles < target {
        m.step().unwrap();
    }
    let at = m.bus.current_cycle;
    assert!(m.bus.drive_input_bit(GPIOA_IDR, 0, level), "PA0 drivable");
    at
}

/// Drive a square wave of `period` cycles with `high` cycles high on PA0,
/// `cycles_total` long, and return the delivered rising and falling cycles.
fn square_wave(
    m: &mut Machine<CortexM>,
    start: u64,
    period: u64,
    high: u64,
    count: usize,
) -> (Vec<u64>, Vec<u64>) {
    let mut rises = Vec::new();
    let mut falls = Vec::new();
    for k in 0..count as u64 {
        rises.push(edge_at(m, start + k * period, true));
        falls.push(edge_at(m, start + k * period + high, false));
    }
    (rises, falls)
}

#[test]
fn pwm_input_mode_measures_period_and_duty_exactly() {
    for (period, high) in [(8_400u64, 2_100u64), (84_000, 58_800), (1_234, 617)] {
        let mut m = machine(" []", "freq-meter.elf");
        let (rises, falls) = square_wave(&mut m, 20_000, period, high, 12);
        // Let the firmware poll the last capture.
        let settle = m.total_cycles + 2_000;
        while m.total_cycles < settle {
            m.step().unwrap();
        }
        let r = results(&m, 6);
        assert!(r[0] >= 8, "period {period}: samples taken {}", r[0]);
        // The last full period the timer saw: between the last two rising
        // edges, in raw cycles (PSC=0). Equality: the edge cycles are the ones
        // this test delivered, and CNT was replayed to exactly those cycles.
        let n = rises.len();
        let measured_period = rises[n - 1] - rises[n - 2];
        let measured_high = falls[n - 2] - rises[n - 2];
        assert_eq!(u64::from(r[1]), measured_period, "period {period}: CCR1");
        assert_eq!(u64::from(r[2]), measured_high, "period {period}: CCR2");
        // And those are the injected numbers up to the instruction boundary
        // the harness could stop at (a few cycles).
        assert!(measured_period.abs_diff(period) <= 8);
        let hz = F401_HZ / period as f64;
        assert!(
            (f64::from(r[3]) - hz).abs() / hz < 0.01,
            "period {period}: {} Hz vs {hz} Hz",
            r[3]
        );
        let duty = 1000.0 * high as f64 / period as f64;
        assert!(
            (f64::from(r[4]) - duty).abs() <= 5.0,
            "period {period}: duty {}‰ vs {duty}‰",
            r[4]
        );
        assert_eq!(r[5], 0, "period {period}: the poll loop kept up");
    }
}

#[test]
fn pwm_input_mode_sees_nothing_when_the_pad_is_not_muxed_to_the_timer() {
    // Negative control: the same waveform on PA0 while the firmware is still
    // held before it muxes PA0 to AF1 is invisible — but the stronger control
    // is a pad the timer never owns. PA4 is not a TIM2 input on the F401.
    let mut m = machine(" []", "freq-meter.elf");
    for k in 0..12u64 {
        while m.total_cycles < 20_000 + k * 8_400 {
            m.step().unwrap();
        }
        m.bus.drive_input_bit(GPIOA_IDR, 4, true);
        while m.total_cycles < 20_000 + k * 8_400 + 2_100 {
            m.step().unwrap();
        }
        m.bus.drive_input_bit(GPIOA_IDR, 4, false);
    }
    let r = results(&m, 6);
    assert_eq!(r[0], 0, "no capture from a non-timer pad: {r:?}");
}

#[test]
fn a_timer_without_the_f4_declaration_gets_no_pad_routes() {
    // Fail-closed: a V2-GPIO part whose timers do not declare
    // `input_capture: stm32f4` must not be routed from the F4 table. The
    // L476 has V2 ports and no declaration.
    let chip = ChipDescriptor::from_file(root().join("configs/chips/stm32l476.yaml")).unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str("name: l4\nchip: unused\nexternal_devices: []\n").unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let idx = bus.find_peripheral_index_by_name("gpioa").unwrap();
    // Mux PA0 to AF1 by hand: still no TIM2_CH1 routing name.
    let mut bus = bus;
    bus.write_u32(0x4002_104C, 1).unwrap(); // RCC_AHB2ENR.GPIOAEN
    bus.write_u32(0x4800_0000, 0xABFF_FFFE).unwrap(); // PA0 = AF
    bus.write_u32(0x4800_0020, 0x1).unwrap(); // AF1
    let routing = bus.peripherals[idx].dev.gpio_routing(0).unwrap();
    assert_eq!(routing.func.as_deref(), Some("AF1"));

    // The F401, declared, names the signal.
    let chip = ChipDescriptor::from_file(root().join("configs/chips/stm32f401.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.write_u32(0x4002_3830, 1).unwrap(); // GPIOAEN
    bus.write_u32(0x4002_0000, 0xA800_0002).unwrap();
    bus.write_u32(0x4002_0020, 0x1).unwrap();
    let idx = bus.find_peripheral_index_by_name("gpioa").unwrap();
    let routing = bus.peripherals[idx].dev.gpio_routing(0).unwrap();
    assert_eq!(routing.func.as_deref(), Some("TIM2_CH1"));
}
