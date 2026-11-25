// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the RASPBERRY PI PICO and PICO 2
//! profiles into the RP2040's SAR converter. The decks already proven on the
//! STM32, WCH, nRF52, micro:bit, AVR and Renesas RA boards run here unchanged:
//! a Beta-parameter NTC divider, a Shockley diode load line and a plain
//! resistor divider name only `board.analog.<pad>_volts`, and the RP2040 /
//! RP2350 descriptors' new `analog_pins:` maps select ADC0..ADC3 on
//! GPIO26..GPIO29.
//!
//! These are the first Raspberry Pi parts in the matrix. Nothing in the deck,
//! solver, router or converter changed: only the chip descriptors gained the
//! pad map, so the test proves the mechanism is per-part data. The two parts
//! instantiate the *same* `rp2040_adc` model at different base addresses and on
//! different interrupt lines (22 on the RP2040, 35 on the RP2350), which this
//! test reads back from each descriptor instead of hardcoding.
//!
//! Each conversion runs the `pico-sdk` register sequence over the board bus:
//! enable `CS`, enable the FIFO in `FCS`, select `AINSEL`, `START_ONCE`, then
//! pop `FIFO` — the path `adc_read()` actually uses, not `RESULT`.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;
use std::sync::atomic::Ordering;

// ADC register offsets, RP2040 datasheet §4.9.5. The RP2350 keeps the layout;
// only the base address and the interrupt line differ, and both are read from
// the descriptor.
const ADC_CS: u64 = 0x00;
const ADC_FCS: u64 = 0x08;
const ADC_FIFO: u64 = 0x0C;
const ADC_INTR: u64 = 0x14;
const ADC_INTE: u64 = 0x18;

/// `CS.EN` — converter powered.
const CS_EN: u32 = 1 << 0;
/// `CS.TS_EN` — temperature-sensor input biased.
const CS_TS_EN: u32 = 1 << 1;
/// `CS.START_ONCE` — write-only, self-clearing.
const CS_START_ONCE: u32 = 1 << 2;
/// `CS.AINSEL` bit position.
const CS_AINSEL_SHIFT: u32 = 12;
/// `FCS.EN` — push every conversion into the FIFO.
const FCS_EN: u32 = 1 << 0;
/// `FCS.EMPTY` — the FIFO holds no samples.
const FCS_EMPTY: u32 = 1 << 8;
/// `FCS.THRESH` bit position; `INTR.FIFO` reads `LEVEL >= THRESH`.
const FCS_THRESH_SHIFT: u32 = 24;

/// `ADC_VREF` on every Pico-class board, in microvolts (datasheet §4.9.4:
/// "the ADC has an internal 3.3 V reference"; the model's `VREF_UV`).
const VREF_MICROVOLTS: i64 = 3_300_000;
/// 12-bit full-scale count.
const FULL_SCALE: i64 = 1 << 12;

/// Parked `b .` guest in SRAM. Both parts execute from SRAM in the simulator,
/// so a conversion can be advanced through the production `Machine` without a
/// firmware image.
const SPIN_PC: u32 = 0x2000_0100;
const VECTOR_TABLE: u32 = 0x2000_0000;
const STACK_TOP: u32 = 0x2000_4000;

/// Beta-parameter NTC resistance, the model card in the fixture.
fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line on the deck's 3.3 V rail, including the
/// diode's series resistance.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// The count the converter must report for a routed level: the model takes
/// whole millivolts and scales them against the 3.3 V reference.
fn expected_code(volts: f64) -> u16 {
    let millivolts = (volts * 1000.0).round() as i64;
    (millivolts * 1_000 * FULL_SCALE / VREF_MICROVOLTS).clamp(0, FULL_SCALE - 1) as u16
}

/// The ADC block as the descriptor declares it.
struct AdcBlock {
    base: u64,
    irq: u32,
}

fn adc_block(chip: &ChipDescriptor) -> AdcBlock {
    let decl = chip
        .peripherals
        .iter()
        .find(|p| p.id == "adc")
        .expect("the chip descriptor declares an ADC");
    AdcBlock {
        base: decl.base_address,
        irq: decl.irq.expect("the ADC declares its interrupt line"),
    }
}

/// A bus built from the profile, plus a Cortex-M parked on `b .` so peripheral
/// ticks and interrupt delivery run through the production machine.
fn board_machine(chip: &ChipDescriptor, manifest: &SystemManifest) -> Machine<CortexM> {
    let mut bus = SystemBus::from_config(chip, manifest).expect("build the board bus");
    // Vector table in SRAM: an IRQ must enter a handler that spins instead of
    // fetching through the unmapped reset window.
    for entry in 0..8u32 {
        let value = if entry == 0 {
            STACK_TOP
        } else {
            (SPIN_PC + 4) | 1
        };
        bus.write_u32(u64::from(VECTOR_TABLE + entry * 4), value)
            .expect("plant a vector table entry");
    }
    bus.write_u16(u64::from(SPIN_PC), 0xe7fe)
        .expect("write b .");
    let (mut cpu, _nvic) = configure_cortex_m(&mut bus);
    cpu.pc = SPIN_PC;
    cpu.sp = STACK_TOP;
    let mut machine = Machine::new(cpu, bus);
    machine.cpu.set_vtor(VECTOR_TABLE);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

/// `pico-sdk`'s `adc_read()`: enable the converter and the FIFO, select the
/// input, start one conversion, then pop the sample.
fn pico_adc_read(machine: &mut Machine<CortexM>, adc: &AdcBlock, channel: u8) -> u16 {
    let bus = &mut machine.bus;
    bus.write_u32(adc.base + ADC_CS, CS_EN).expect("CS.EN");
    bus.write_u32(adc.base + ADC_FCS, FCS_EN).expect("FCS.EN");
    bus.write_u32(
        adc.base + ADC_CS,
        CS_EN | (u32::from(channel) << CS_AINSEL_SHIFT),
    )
    .expect("CS.AINSEL");
    bus.write_u32(
        adc.base + ADC_CS,
        CS_EN | (u32::from(channel) << CS_AINSEL_SHIFT) | CS_START_ONCE,
    )
    .expect("CS.START_ONCE");
    assert_eq!(
        bus.read_u32(adc.base + ADC_FCS).expect("read FCS") & FCS_EMPTY,
        0,
        "a START_ONCE conversion must leave a sample in the FIFO"
    );
    (bus.read_u32(adc.base + ADC_FIFO).expect("pop FIFO") & 0x0FFF) as u16
}

/// ISPR bit for an interrupt line, read from the machine's NVIC.
fn nvic_pending(machine: &Machine<CortexM>, line: u32) -> bool {
    let nvic = machine
        .bus
        .nvic
        .as_ref()
        .expect("Cortex-M bus carries an NVIC");
    let word = nvic.ispr[(line / 32) as usize].load(Ordering::SeqCst);
    word & (1 << (line % 32)) != 0
}

/// Load a Pico profile, overlay the generic deck's `cosim_models`, and build
/// the machine and co-simulation.
fn bench(system: &str, deck: &str) -> (ChipDescriptor, Machine<CortexM>, CosimSession, AdcBlock) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path).expect("load Pico profile");
    let deck = SystemManifest::from_file(root(deck)).expect("load the generic analog deck");
    profile.cosim_models = deck.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the chip descriptor");

    for (pad, channel) in [("GP26", 0), ("GP27", 1), ("GP28", 2), ("GP29", 3)] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{pad} must resolve to ADC channel {channel}"
        );
        assert_eq!(chip.analog_pins[pad].peripheral, "adc");
    }
    // The temperature sensor is not a GPIO pad: routing a board net at it would
    // bias the die sensor, so it must stay off the map.
    assert!(
        !chip.analog_pins.contains_key("GP30"),
        "no pad past GP29 exists on the RP2040/RP2350A pad banks"
    );

    let adc = adc_block(&chip);
    let machine = board_machine(&chip, &profile);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build the analog co-simulation")
        .expect("the deck carries analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic decks must resolve GP26/GP27/GP28 to ADC channels 0/1/2"
    );
    (chip, machine, session, adc)
}

/// Every shipped profile whose chip is an RP2040 or an RP2350A. The deck names
/// pads, not a board, so one netlist per family serves all of them; each board
/// is listed here so a profile that loses its pad map is a red test.
const RP2040_DECK: &str = "examples/pico/analog-ntc-diode-system.yaml";
const RP2350_DECK: &str = "examples/pico2/analog-ntc-diode-system.yaml";
const RP2040_PROFILES: &[&str] = &[
    "configs/systems/pico.yaml",
    "configs/systems/pico-bme280.yaml",
    "configs/systems/pico-selftest.yaml",
    "configs/systems/pico-w-bme280.yaml",
    "configs/systems/rp2040-pico.yaml",
    "configs/systems/adafruit-feather-rp2040.yaml",
    "configs/systems/adafruit-feather-rp2040-bme280.yaml",
];
const RP2350_PROFILES: &[&str] = &[
    "configs/systems/pico2.yaml",
    "configs/systems/pico2-bme280.yaml",
    "configs/systems/rp2350-zero.yaml",
    "configs/systems/rpi-pico-2-w.yaml",
    "configs/systems/rpi-pico-2-w-bme280.yaml",
    "configs/systems/adafruit-feather-rp2350.yaml",
];

/// One temperature point: the solved NTC voltage, the solved diode voltage and
/// the NTC ADC count the firmware reads back.
type Sample = (f64, f64, u16);

/// One board run: its profile path and the sample at each temperature.
type ProfileSamples<'a> = (&'a str, Vec<Sample>);

/// NTC, diode and divider decks solve, route and convert on every Pico-family
/// profile through that board's own descriptor.
#[test]
fn every_pico_profile_converts_the_generic_ntc_and_diode_decks() {
    let expected_diode_v = diode_load_line_voltage();
    let expected_divider_v = 3.3 * 4_700.0 / (10_000.0 + 4_700.0);
    let mut family_samples: Vec<Vec<ProfileSamples>> = vec![Vec::new(), Vec::new()];

    for (family_index, (deck, profiles, family_irq)) in [
        (RP2040_DECK, RP2040_PROFILES, 22u32),
        (RP2350_DECK, RP2350_PROFILES, 35),
    ]
    .into_iter()
    .enumerate()
    {
        for &system in profiles {
            let (_chip, mut machine, mut session, adc) = bench(system, deck);
            assert_eq!(adc.irq, family_irq, "{system}: ADC interrupt line");
            let mut cycles = 0;
            let mut samples = Vec::new();

            for temperature_c in [-20.0_f64, 25.0, 85.0] {
                session
                    .set_signal_number("ui.ntc.temperature_c", temperature_c)
                    .expect("set generic NTC temperature");
                cycles += session.cycles_until_boundary(cycles);
                let (steps, errors) = session
                    .advance_to(cycles, &mut machine.bus)
                    .expect("solve and route the three analog decks");
                assert_eq!(
                    steps.len(),
                    3,
                    "{system}: all three decks cross the boundary"
                );
                assert!(
                    errors.is_empty(),
                    "{system}: unexpected route errors: {errors:?}"
                );

                let ntc_v = signal_voltage(&session, "board.analog.gp26_volts");
                let expected_ntc_v =
                    3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
                assert!(
                    (ntc_v - expected_ntc_v).abs() < 1e-9,
                    "{system} at {temperature_c} C: the Beta divider on GP26 should be \
                     {expected_ntc_v} V, got {ntc_v} V"
                );
                let diode_v = signal_voltage(&session, "board.analog.gp27_volts");
                assert!(
                    (diode_v - expected_diode_v).abs() < 2e-5,
                    "{system}: the Shockley load line on GP27 should be \
                     {expected_diode_v} V, got {diode_v} V"
                );
                let divider_v = signal_voltage(&session, "board.analog.gp28_volts");
                assert!(
                    (divider_v - expected_divider_v).abs() < 1e-9,
                    "{system}: the GP28 divider should be {expected_divider_v} V, \
                     got {divider_v} V"
                );

                // The three pads carry three different levels, so a swapped channel
                // assignment cannot pass: 25 C NTC is exactly half scale, the
                // divider is ~1.05 V and the diode ~0.5 V.
                assert_eq!(
                    pico_adc_read(&mut machine, &adc, 0),
                    expected_code(ntc_v),
                    "{system}: ADC0 (GP26) must report the solved NTC voltage"
                );
                assert_eq!(
                    pico_adc_read(&mut machine, &adc, 1),
                    expected_code(diode_v),
                    "{system}: ADC1 (GP27) must report the solved diode voltage"
                );
                assert_eq!(
                    pico_adc_read(&mut machine, &adc, 2),
                    expected_code(divider_v),
                    "{system}: ADC2 (GP28) must report the solved divider voltage"
                );
                // ADC3 (GP29) has no deck: the model's documented undriven level is
                // 0 V, so a nonzero code would mean a routed input bled sideways.
                assert_eq!(
                    pico_adc_read(&mut machine, &adc, 3),
                    0,
                    "{system}: an unrouted channel must keep the model's 0 V default"
                );

                samples.push((ntc_v, diode_v, expected_code(ntc_v)));
            }

            assert!(
                samples[2].0 > samples[1].0 && samples[1].0 > samples[0].0,
                "{system}: the NTC divider voltage must rise with temperature: {samples:?}"
            );
            assert!(
                samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
                "{system}: ADC counts must track the solved voltage: {samples:?}"
            );
            assert_eq!(
                samples[1].2,
                expected_code(1.65),
                "{system}: the 25 C NTC divider is exactly half of the 3.3 V reference"
            );
            family_samples[family_index].push((system, samples));
        }
    }

    // Every profile of a family is the same silicon and the same rail, so they
    // must produce the same solved values; and the two parts are different
    // silicon with the same 3.3 V/12-bit converter, so the families must agree
    // too. Both comparisons catch a per-chip fudge in the model or router.
    let mut baselines: Vec<&Sample> = Vec::new();
    for (family_index, samples_by_profile) in family_samples.iter().enumerate() {
        let (baseline_profile, baseline) = samples_by_profile
            .first()
            .expect("the family lists at least one profile");
        for (profile, samples) in samples_by_profile {
            for (index, (solved, reference)) in samples.iter().zip(baseline.iter()).enumerate() {
                assert!(
                    (solved.0 - reference.0).abs() < 1e-12
                        && (solved.1 - reference.1).abs() < 1e-12
                        && solved.2 == reference.2,
                    "{profile} sample {index} must match {baseline_profile} in family \
                     {family_index}: {solved:?} vs {reference:?}"
                );
            }
        }
        baselines.push(&baseline[1]);
    }
    let (rp2040, rp2350) = (baselines[0], baselines[1]);
    assert!(
        (rp2040.0 - rp2350.0).abs() < 1e-12
            && (rp2040.1 - rp2350.1).abs() < 1e-12
            && rp2040.2 == rp2350.2,
        "the same deck must solve and convert identically on both parts: \
         {rp2040:?} vs {rp2350:?}"
    );
}

/// The die temperature sensor is selected by `CS.TS_EN`, not by a pad: biased
/// it converts the datasheet transfer function at 27 C, unbiased it reads 0.
#[test]
fn pico_temperature_input_is_gated_by_ts_en_and_not_reachable_from_a_pad() {
    let (_chip, mut machine, _session, adc) = bench(
        "configs/systems/pico.yaml",
        "examples/pico/analog-ntc-diode-system.yaml",
    );
    let bus = &mut machine.bus;

    // Unbiased: an input with no source converts 0.
    bus.write_u32(adc.base + ADC_FCS, FCS_EN).expect("FCS.EN");
    bus.write_u32(
        adc.base + ADC_CS,
        CS_EN | (4 << CS_AINSEL_SHIFT) | CS_START_ONCE,
    )
    .expect("convert the temperature input");
    assert_eq!(
        bus.read_u32(adc.base + ADC_FIFO).expect("pop FIFO") & 0x0FFF,
        0,
        "CS.TS_EN clear leaves the temperature sensor unpowered, so it converts 0"
    );

    // Biased at the model's 27 C ambient: 0.706 V against the 3.3 V reference.
    bus.write_u32(
        adc.base + ADC_CS,
        CS_EN | CS_TS_EN | (4 << CS_AINSEL_SHIFT) | CS_START_ONCE,
    )
    .expect("convert the biased temperature input");
    let code = (bus.read_u32(adc.base + ADC_FIFO).expect("pop FIFO") & 0x0FFF) as u16;
    assert_eq!(
        code,
        expected_code(0.706),
        "the datasheet transfer function puts 27 C at 0.706 V, not at a magic code"
    );
}

/// The same `rp2040_adc` model sits on each part's own interrupt line: the
/// descriptor's `irq:` decides which NVIC line the FIFO threshold pends, and a
/// masked ADC must not pend at all.
#[test]
fn every_pico_profile_pends_its_fifo_threshold_on_the_descriptor_irq() {
    for (deck, profiles, family_irq) in [
        (RP2040_DECK, RP2040_PROFILES, 22u32),
        (RP2350_DECK, RP2350_PROFILES, 35),
    ] {
        for &system in profiles {
            let (_chip, mut machine, _session, adc) = bench(system, deck);
            assert_eq!(
                adc.irq, family_irq,
                "{system}: the descriptor's ADC irq must stay {family_irq}"
            );

            // Threshold 1: one queued sample asserts INTR.FIFO.
            let bus = &mut machine.bus;
            bus.write_u32(adc.base + ADC_FCS, FCS_EN | (1 << FCS_THRESH_SHIFT))
                .expect("FCS.EN with THRESH = 1");
            bus.write_u32(adc.base + ADC_INTE, 1)
                .expect("unmask ADC_IRQ_FIFO");
            bus.write_u32(adc.base + ADC_CS, CS_EN | CS_START_ONCE)
                .expect("start one conversion");
            assert_ne!(
                bus.read_u32(adc.base + ADC_INTR).expect("read INTR") & 1,
                0,
                "{system}: one sample at a threshold of one must assert INTR.FIFO"
            );
            machine.run(Some(256)).expect("advance the machine");
            assert!(
                nvic_pending(&machine, family_irq),
                "{system}: ADC_IRQ_FIFO must pend NVIC line {family_irq}"
            );
            assert!(
                !nvic_pending(&machine, family_irq ^ 1),
                "{system}: the neighbouring line must stay quiet"
            );

            // The negative direction: with INTE clear the same level must not pend,
            // so the test above cannot be satisfied by anything that pends blindly.
            let (_chip, mut masked, _session, adc) = bench(system, deck);
            let bus = &mut masked.bus;
            bus.write_u32(adc.base + ADC_FCS, FCS_EN | (1 << FCS_THRESH_SHIFT))
                .expect("FCS.EN with THRESH = 1");
            bus.write_u32(adc.base + ADC_INTE, 0)
                .expect("mask the ADC line");
            bus.write_u32(adc.base + ADC_CS, CS_EN | CS_START_ONCE)
                .expect("start one conversion");
            masked.run(Some(256)).expect("advance the machine");
            assert!(
                !nvic_pending(&masked, adc.irq),
                "{system}: a masked ADC must not pend line {}",
                adc.irq
            );
        }
    }
}

/// A pad that is not an analog input is refused at binding time. Without this
/// the router could fall back to the same channel number of any ADC on the bus
/// and report a plausible level from the wrong pin.
#[test]
fn a_non_analog_pad_is_refused_instead_of_guessing_a_channel() {
    let system_path = root("configs/systems/pico.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Pico profile");
    let deck = SystemManifest::from_file(root("examples/pico/analog-ntc-diode-system.yaml"))
        .expect("load the generic analog deck");
    let mut models = deck.cosim_models;
    models[0]
        .outputs
        .insert("v_sense".to_string(), "board.analog.gp25_volts".to_string());
    profile.cosim_models = models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the chip descriptor");
    let machine = board_machine(&chip, &profile);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build the analog co-simulation")
        .expect("the deck carries analog devices");

    assert_eq!(
        session.binding_errors(),
        &[RoutingError::NoAdcChannel {
            path: "board.analog.gp25_volts".to_string(),
            pad: "gp25".to_string(),
        }],
        "GP25 is the LED pad, not an ADC input: the router must say so"
    );
}
