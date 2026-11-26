// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F103C8T6 Blue Pill hardware PWM, as stock STM32duino `analogWrite()`
//! programs it: TIM3_CH1 on PA6 and TIM2_CH2 on PA1.
//!
//! # Why this test exists
//!
//! The F1 fixed-map timer-output route (`SystemBus::wire_stm32_timer_output_pads`,
//! RM0008 §9.3.7 Tables 43-46) had no core test: `stm32_v2_timer_output_pad`
//! covers the V2/AF-map families and the CH32V003 test its own fixed map. The
//! Arduino matrix's L6 level did count 61 edges on PA0 for a NUCLEO-F103RB
//! (2026-11-21), but a `min_edges: 20` oracle only proves the route fires: it
//! says nothing about frequency, duty, channel independence, or a 0 % channel,
//! and the matrix's Blue Pill row has no committed evidence directory. On the
//! #3 part in the measured demand list (STM32F103, 17 %, JetBrains State of
//! Embedded 2025) the thing people do with it is drive something with a PWM
//! pin: an LED, a servo, an L298N.
//!
//! # What is measured, not assumed
//!
//! The fixture is an unmodified STM32duino sketch (`analogWriteFrequency()` +
//! `analogWrite()`), so the register writes are the library's, not the test's:
//! `initVariant`, the PLL clock tree, `HardwareTimer::setMode(PWM1)`,
//! `setOverflow(1000 Hz)`, `setCaptureCompare(…,
//! RESOLUTION_8B_COMPARE_FORMAT)`. From the captured pad waveform the test
//! reads back the FREQUENCY and the DUTY, which is what a customer's scope
//! shows:
//!
//! * PA6 = TIM3_CH1 at `analogWrite(PA6, 64)` → 64/255 = 25.10 % of 1 kHz;
//! * PA1 = TIM2_CH2 at `analogWrite(PA1, 192)` → 192/255 = 75.29 % of 1 kHz,
//!   proving the two channels are independent and that a second timer is
//!   routed too;
//! * PB12, a plain `digitalWrite()` pin, shows no edges: the capture counts
//!   real timer outputs, not every pad the sketch touches;
//! * `analogWrite(PA6, 0)` drops PA6 low and stops its edges while PA1 keeps
//!   192/255 — a 0 % channel is the negative control for "the pad still
//!   pulses because the model never applied CCR".
//!
//! The odd-looking fractions are the library's, not a model error:
//! `setCaptureCompare(…, RESOLUTION_8B_COMPARE_FORMAT)` programs
//! `CCR = (ARR + 1) × value / 255`, so an 8-bit `analogWrite(v)` is `v/255`.
//!
//! # The clock
//!
//! `examples/stm32f103-blue-pill/pwm-system.yaml` runs at 72 MHz, the clock
//! every Blue Pill project (and STM32duino's `SystemClock_Config()`) selects
//! through the 8 MHz crystal and the PLL. `cpu_hz` is a static per-run value in
//! this engine, and the board profile `configs/systems/stm32f103-blue-pill.yaml`
//! stays at the 8 MHz HSI reset clock its own bare-metal smoke boots at. The
//! 1 kHz assertion is therefore also a check of that declaration: a firmware
//! clocked 9× off would land at 9 kHz or 111 Hz.
//!
//! # Not claimed
//!
//! * TIM1 complementary outputs (plain `PA7` is `TIM1_CH1N` on this variant):
//!   not modelled, so a sketch that sets `analogWrite(PA7, …)` must not expect
//!   a waveform here. Use PA6, PA1, or another plain channel.
//! * The AFIO remaps (`AFIO_MAPR`): only the default fixed map is wired.
//! * Dead time, break input, and the advanced-timer output stage.
//!
//! Rebuild the guest:
//!
//!     cd tests/fixtures/stm32duino-f103-bluepill-pwm && pio run -e bluepill
//!     arm-none-eabi-strip -o ../stm32duino-f103-bluepill-pwm.elf \
//!         .pio/build/bluepill/firmware.elf

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::console::ConsoleCapture;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::logic_capture::{LogicEdge, LogicSource};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type Cm = Machine<CortexM>;

const CPU_HZ: f64 = 72_000_000.0;
/// PA6: TIM3_CH1, L298N ENA.
const PA6_CH: u32 = 0;
/// PA1: TIM2_CH2.
const PA1_CH: u32 = 1;
/// PB12: plain digital output, the flat-pad control.
const PB12_CH: u32 = 2;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Rig {
    machine: Cm,
    uart: Arc<Mutex<Vec<u8>>>,
}

impl Rig {
    fn uart_text(&self) -> String {
        String::from_utf8_lossy(&self.uart.lock().unwrap()).into_owned()
    }

    /// Advance until the guest's UART contains `marker`, returning the cycle it
    /// was first seen at. `None` when `max_cycles` elapse first.
    fn run_until(&mut self, marker: &str, max_cycles: u64) -> Option<u64> {
        let mut spent = 0;
        while spent < max_cycles {
            if self.uart_text().contains(marker) {
                return Some(self.machine.logic_now_cycle());
            }
            self.machine
                .advance(AdvanceRequest::run(Some(100_000)))
                .expect("advance");
            spent += 100_000;
        }
        None
    }

    fn advance(&mut self, cycles: u64) {
        self.machine
            .advance(AdvanceRequest::run(Some(cycles)))
            .expect("advance");
    }

    fn edges(&mut self) -> Vec<LogicEdge> {
        self.machine.logic_read_edges(0).edges
    }
}

fn rig() -> Rig {
    let chip = ChipDescriptor::from_file(root().join("configs/chips/stm32f103.yaml"))
        .expect("stm32f103 chip descriptor");
    let manifest =
        SystemManifest::from_file(root().join("examples/stm32f103-blue-pill/pwm-system.yaml"))
            .expect("Blue Pill PWM board profile");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    // The board console: `debug_uart: uart1`, exactly what the CLI wires.
    let console = ConsoleCapture::for_manifest(&manifest);
    let uart = console.heard_sink();
    bus.attach_host_console_echo(console.tapped(), uart.clone(), false)
        .expect("console");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let elf = std::fs::read(root().join("tests/fixtures/stm32duino-f103-bluepill-pwm.elf"))
        .expect("committed STM32duino fixture");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse fixture");
    machine.load_firmware(&image).expect("load fixture");

    let gpioa = machine
        .bus
        .find_peripheral_index_by_name("gpioa")
        .expect("gpioa");
    let gpiob = machine
        .bus
        .find_peripheral_index_by_name("gpiob")
        .expect("gpiob");
    machine.logic_watch(&[
        Some(LogicSource::Pad {
            peripheral: gpioa,
            pin: 6,
        }),
        Some(LogicSource::Pad {
            peripheral: gpioa,
            pin: 1,
        }),
        Some(LogicSource::Pad {
            peripheral: gpiob,
            pin: 12,
        }),
    ]);
    Rig { machine, uart }
}

/// Frequency and duty of one channel between two ABSOLUTE cycles, measured the
/// way a scope measures: whole periods only. The first and last rising edges
/// inside the window bound the measurement, so a partial period at either end
/// cannot bias the duty.
fn wave(edges: &[LogicEdge], ch: u32, lo: u64, hi: u64) -> (f64, f64) {
    let w: Vec<&LogicEdge> = edges
        .iter()
        .filter(|e| e.ch == ch && e.cycle >= lo && e.cycle <= hi)
        .collect();
    let rises: Vec<u64> = w.iter().filter(|e| e.value).map(|e| e.cycle).collect();
    assert!(
        rises.len() >= 3,
        "channel {ch}: {} edges and {} rising edges in [{lo}, {hi}) — too few to measure",
        w.len(),
        rises.len()
    );
    let span = rises[rises.len() - 1] - rises[0];
    let periods = (rises.len() - 1) as f64;
    let period_cycles = span as f64 / periods;
    // Every rising edge is followed by exactly one falling edge; a channel that
    // ends the window high has an incomplete last period, which `span` already
    // excludes from both sums.
    let high = high_time(&w);
    let duty = high as f64 / ((rises.len() - 1) as f64 * period_cycles);
    (CPU_HZ / period_cycles, duty)
}

/// Sum the rising→falling spans that start before the last rising edge.
fn high_time(w: &[&LogicEdge]) -> u64 {
    let last_rise = w.iter().rposition(|e| e.value).expect("a rising edge");
    (0..last_rise)
        .filter(|&i| w[i].value)
        .map(|i| w[i + 1].cycle - w[i].cycle)
        .sum()
}

fn assert_close(what: &str, got: f64, want: f64, tol: f64) {
    assert!(
        (got - want).abs() <= tol,
        "{what}: measured {got:.4}, expected {want:.4} ± {tol}"
    );
}

#[test]
fn blue_pill_analogwrite_pwm_reaches_pa6_and_pa1_at_1_khz() {
    let mut r = rig();
    let ready = r
        .run_until("PA1_DUTY 192", 3_000_000)
        .unwrap_or_else(|| panic!("the sketch never printed READY; UART:\n{}", r.uart_text()));
    let window_end = ready + 2_000_000;
    r.advance(2_000_000);

    let edges = r.edges();
    let (pa6_hz, pa6_duty) = wave(&edges, PA6_CH, ready, window_end);
    let (pa1_hz, pa1_duty) = wave(&edges, PA1_CH, ready, window_end);

    // 1000 Hz is the library call's own frequency; it comes out of PSC/ARR as
    // the library computes them from the modelled RCC clock.
    assert_close("PA6 (TIM3_CH1) frequency", pa6_hz, 1000.0, 1.0);
    assert_close("PA1 (TIM2_CH2) frequency", pa1_hz, 1000.0, 1.0);
    // analogWrite(64) is 64/255 and analogWrite(192) is 192/255: the library
    // divides by 255, not 256 (HardwareTimer.cpp, setCaptureCompare).
    assert_close("PA6 duty", pa6_duty, 64.0 / 255.0, 0.001);
    assert_close("PA1 duty", pa1_duty, 192.0 / 255.0, 0.001);
    eprintln!(
        "PA6 TIM3_CH1: {pa6_hz:.1} Hz at {:.2} %; PA1 TIM2_CH2: {pa1_hz:.1} Hz at {:.2} %",
        pa6_duty * 100.0,
        pa1_duty * 100.0
    );

    // The flat control: PB12 was driven high by digitalWrite() during setup and
    // owns no timer channel, so once the sketch is up it must hold still — the
    // capture counts timer output, not every pad the sketch ever touched.
    let pb12 = edges
        .iter()
        .filter(|e| e.ch == PB12_CH && e.cycle >= ready && e.cycle <= window_end)
        .count();
    assert_eq!(
        pb12, 0,
        "PB12 is a plain digital output but the capture reported {pb12} edges after setup"
    );
}

#[test]
fn analogwrite_zero_drops_pa6_while_pa1_keeps_its_duty() {
    let mut r = rig();
    r.run_until("PA1_DUTY 192", 3_000_000)
        .unwrap_or_else(|| panic!("the sketch never printed READY; UART:\n{}", r.uart_text()));
    let stop = r
        .run_until("PA6_DUTY 0", 12_000_000)
        .unwrap_or_else(|| panic!("the sketch never printed STOP; UART:\n{}", r.uart_text()));
    // Three 1 kHz periods after the marker: enough for a channel that failed to
    // fall to show at least one edge, and for PA1 to show at least three.
    let tail_start = stop + 100_000;
    let tail_end = stop + 400_000;
    r.advance(400_000);

    let edges = r.edges();
    let pa6_edges = edges
        .iter()
        .filter(|e| e.ch == PA6_CH && e.cycle >= tail_start && e.cycle <= tail_end)
        .count();
    assert_eq!(
        pa6_edges, 0,
        "analogWrite(PA6, 0) must leave PA6 static, saw {pa6_edges} edges"
    );
    let pa6_level = r
        .machine
        .read_logic_level(LogicSource::Pad {
            peripheral: r
                .machine
                .bus
                .find_peripheral_index_by_name("gpioa")
                .unwrap(),
            pin: 6,
        })
        .expect("PA6 is watched");
    assert!(!pa6_level, "analogWrite(PA6, 0) leaves the pad low");

    // PA1 was never touched: it must still be the 75 % 1 kHz waveform.
    let (pa1_hz, pa1_duty) = wave(&edges, PA1_CH, tail_start, tail_end);
    assert_close("PA1 frequency after PA6 stops", pa1_hz, 1000.0, 1.0);
    assert_close("PA1 duty after PA6 stops", pa1_duty, 192.0 / 255.0, 0.001);
}
