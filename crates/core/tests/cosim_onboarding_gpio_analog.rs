// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive a GPIO input on every imported onboarding
//! profile (`configs/systems/onboarding/*.yaml`) that models one.
//!
//! The ADC sweep in `cosim_onboarding_analog.rs` reaches only the 12 profiles
//! whose converter LabWired can run. This is the other endpoint the coverage
//! inventory accepts, a voltage-thresholded digital input, and it is just as
//! board-agnostic: the port is the first peripheral each built bus reports as
//! a GPIO input, the same Beta NTC divider and Shockley diode load line are
//! routed to `board.gpio_in.<port>.0` / `.1`, the solved voltages are checked
//! against independent references, and the levels are read back twice. Once
//! through `Peripheral::read_gpio_input`, and once with a firmware load from
//! the port's input register.
//!
//! The register is where it gets honest. Most Renode-imported GPIO types have
//! no dedicated LabWired model and run on the STM32F1 placeholder map
//! (`SystemBus::gpio_layout_for`, case 3b), so their input reads back at the
//! placeholder IDR (+0x08). That is NOT the silicon register for that IP, and
//! [`PLACEHOLDER_TYPES`] says so. Only [`SILICON_INPUT_REGISTER`] types are
//! claimed at the silicon offset. GPIO thresholds are simulation assumptions
//! wherever the descriptor declares none, and this is not ADC support.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::{Path, PathBuf};

const NTC_PIN: u8 = 0;
const DIODE_PIN: u8 = 1;

/// GPIO types whose LabWired model serves the silicon register map, with the
/// input register's offset in that map and its source.
const SILICON_INPUT_REGISTER: &[(&str, u64, &str)] = &[
    ("stm32f1gpioport", 0x08, "RM0008 §9.2.3 GPIOx_IDR"),
    ("gpio", 0x10, "RM0368 §8.4.5 GPIOx_IDR (profile stm32v2)"),
    ("imxrt_gpio", 0x08, "IMXRT1060RM §12.5 GPIO_PSR"),
    ("nrf52840_gpio", 0x510, "nRF52840 PS §6.9.2 IN"),
    ("sam4s_pio", 0x3C, "SAM4S DS §31.7 PIO_PDSR"),
    ("miv_coregpio", 0x90, "CoreGPIO handbook InputRegister"),
    // The AVR CPU answers an IN/LDS of PINx with this mirror's input bits.
    (
        "avr_gpio",
        0x00,
        "PINx (DS8271 §14.4, DS2549 §13.4), via the CPU IO mirror",
    ),
];

/// Placeholder IDR offset: GPIO types LabWired does not model yet run on the
/// STM32F1 map, so the level is firmware-visible here and nowhere else.
const PLACEHOLDER_INPUT_OFFSET: u64 = 0x08;

/// Imported GPIO types running on that placeholder. A type leaving this list
/// for a real model must move to [`SILICON_INPUT_REGISTER`].
const PLACEHOLDER_TYPES: &[&str] = &[
    "ambiqapollo4_gpio",
    "andesatcgpio100",
    "arm_ahb_gpio",
    "cc2538_gpio",
    "designware_apb_gpio",
    "efmgpioport",
    "efr32_gpioport",
    "gaisler_gpio",
    "litex_gpio",
    "max32650_gpio",
    "mpfs_gpio",
    "murax_gpio",
    "npcx_gpio",
    "nxp_imx_gpio",
    "opentitan_gpio",
    "pulp_apb_gpio",
    "quark_gpiocontroller",
    "renesasda14_gpio",
    "renesasra_gpiomisc",
    "samd21_gpio",
    "sifive_gpio",
    "stm32_gpioport",
    "xilinxgpiops",
];

/// Profiles whose port takes the routed level (`read_gpio_input` follows it)
/// but where a firmware load from the port's input register does not see it.
/// Fixing one has to move it out of this list.
const INPUT_REGISTER_UNREACHED: &[(&str, &str)] = &[
    // Cortex-M33 parts with GPIO at 0x4202_xxxx: the bus answers that window
    // as the Cortex-M3/M4 peripheral bit-band alias, which the M33 lacks.
    ("nucleo_wba52cg", "bit-band alias shadows GPIOA"),
    ("ramn", "bit-band alias shadows GPIOA"),
    ("stm32l552", "bit-band alias shadows GPIOA"),
    ("stm32wba52", "bit-band alias shadows GPIOA"),
    // i.MX 8M Plus: every load in the GPIO1 window returns 0xFFFF_FFFF.
    ("nxp-frdm-imx8mplus", "GPIO1 window reads all ones"),
    ("nxp-mimx8ml", "GPIO1 window reads all ones"),
];

/// Profiles covered here: a port takes the routed level.
const COVERED: usize = 133;
/// Covered profiles read back at a [`SILICON_INPUT_REGISTER`] offset.
const SILICON_REGISTER: usize = 31;
/// Profiles whose descriptor declares no GPIO input and no convertible ADC:
/// nothing analog can reach them yet.
const NO_ANALOG_ENDPOINT: usize = 112;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line, including the diode's series resistance.
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn onboarding_profiles() -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root("configs/systems/onboarding"))
        .expect("read onboarding systems")
        .map(|entry| entry.expect("onboarding entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    paths
}

fn generic_netlists(port: &str) -> SystemManifest {
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pin) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", NTC_PIN),
            "diode_load_line" => ("v_diode", DIODE_PIN),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.gpio_in.{port}.{pin}"));
    }
    analog
}

/// The first peripheral, in bus order, that reports a GPIO input on both pins.
fn first_input_port(bus: &SystemBus) -> Option<usize> {
    bus.peripherals.iter().position(|p| {
        p.dev.read_gpio_input(NTC_PIN).is_some() && p.dev.read_gpio_input(DIODE_PIN).is_some()
    })
}

/// Absolute address of the port's input register, from its declared type.
fn input_register(chip: &ChipDescriptor, port: &str, base: u64) -> (u64, bool) {
    let cfg = chip
        .peripherals
        .iter()
        .find(|p| p.id == port)
        .expect("port comes from the descriptor");
    let kind = cfg.r#type.to_ascii_lowercase();
    // A window based part-way into its block (Nordic ports at OUT) says so.
    let window = cfg
        .config
        .get("reg_offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if let Some((_, offset, _)) = SILICON_INPUT_REGISTER.iter().find(|(t, ..)| *t == kind) {
        return (base - window + offset, true);
    }
    assert!(
        PLACEHOLDER_TYPES.contains(&kind.as_str()),
        "{}: GPIO type {kind} is unclassified; add it to SILICON_INPUT_REGISTER or PLACEHOLDER_TYPES",
        chip.name
    );
    (base + PLACEHOLDER_INPUT_OFFSET, false)
}

/// What firmware writes before the pins read as inputs.
fn bring_up_inputs(bus: &mut SystemBus, chip: &ChipDescriptor, port: &str, base: u64) {
    let kind = chip
        .peripherals
        .iter()
        .find(|p| p.id == port)
        .map(|p| p.r#type.to_ascii_lowercase());
    if kind.as_deref() == Some("miv_coregpio") {
        // CoreGPIO masks InputRegister bits whose CONFIG_X.INREG is clear.
        for pin in [NTC_PIN, DIODE_PIN] {
            bus.write_u32(base + 4 * u64::from(pin), 1 << 1)
                .expect("CoreGPIO CONFIG_X.INREG");
        }
    }
}

struct Outcome {
    silicon_register: bool,
}

fn assert_port_routes_generic_analog(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
    port_index: usize,
    probe: &SystemBus,
) -> Outcome {
    let port = probe.peripherals[port_index].name.clone();
    let base = probe.peripherals[port_index].base;
    let tag = format!("{board}:{port}");
    let mut profile = base_profile.clone();
    profile.cosim_models = generic_netlists(&port).cosim_models;
    let mut bus = SystemBus::from_config(chip, &profile).expect("build onboarding bus");
    bring_up_inputs(&mut bus, chip, &port, base);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{tag}: both netlists bind to pins {NTC_PIN}/{DIODE_PIN}"
    );

    let (register, silicon_register) = input_register(chip, &port, base);
    let unreached = INPUT_REGISTER_UNREACHED.iter().any(|(b, _)| *b == board);
    let io_v = chip.io_voltage_v.expect("io voltage set before the bus");
    let expected_diode_v = diode_load_line_voltage();
    assert!(
        expected_diode_v < 0.3 * io_v,
        "{tag}: the diode load line sits below VIL"
    );
    let ntc_path = format!("board.gpio_in.{port}.{NTC_PIN}");
    let diode_path = format!("board.gpio_in.{port}.{DIODE_PIN}");
    let mut register_tracked = true;
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // inside the band: holds high
        (80.0, false),
        (25.0, false), // inside the band: holds low
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both netlists");
        assert_eq!(steps.len(), 2, "{tag}: both netlists solve each step");
        assert!(errors.is_empty(), "{tag}: route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, &ntc_path);
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{tag}: at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, &diode_path);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{tag}: Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let dev = &bus.peripherals[port_index].dev;
        assert_eq!(
            dev.read_gpio_input(NTC_PIN),
            Some(expected_ntc_level),
            "{tag}: NTC pin follows the solved voltage with Schmitt hysteresis at {temperature_c} °C"
        );
        assert_eq!(
            dev.read_gpio_input(DIODE_PIN),
            Some(false),
            "{tag}: diode pin reads low below VIL"
        );

        let word = bus
            .read_u32(register)
            .expect("load the port input register");
        register_tracked &=
            (word & (1 << NTC_PIN) != 0) == expected_ntc_level && word & (1 << DIODE_PIN) == 0;
    }
    if unreached {
        assert!(
            !register_tracked,
            "{tag}: the input register at {register:#x} now tracks the pins; remove it from INPUT_REGISTER_UNREACHED"
        );
        if INPUT_REGISTER_UNREACHED
            .iter()
            .any(|(b, why)| *b == board && why.starts_with("bit-band"))
        {
            assert!(
                bus.bit_band_enabled && (0x4200_0000..0x4400_0000).contains(&register),
                "{tag}: diagnosis is the bit-band alias over {register:#x}"
            );
        }
    } else {
        assert!(
            register_tracked,
            "{tag}: firmware load from {register:#x} must track both pins"
        );
    }
    Outcome {
        silicon_register: silicon_register && !unreached,
    }
}

#[test]
fn every_onboarding_profile_with_a_gpio_input_takes_routed_ntc_and_diode_levels() {
    let mut covered = Vec::new();
    let mut silicon = 0;
    let mut no_endpoint = 0;
    for system_path in onboarding_profiles() {
        let board = system_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let profile =
            SystemManifest::from_file(&system_path).unwrap_or_else(|e| panic!("load {board}: {e}"));
        let mut chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
            .unwrap_or_else(|e| panic!("load {board} descriptor: {e}"));
        // Simulation assumptions where the import records nothing; no
        // silicon threshold is claimed.
        chip.io_voltage_v.get_or_insert(3.3);
        chip.gpio_input_thresholds
            .get_or_insert(GpioInputThresholds { vil: 0.3, vih: 0.7 });
        let probe = SystemBus::from_config(&chip, &profile)
            .unwrap_or_else(|e| panic!("build {board} bus: {e}"));
        let Some(port_index) = first_input_port(&probe) else {
            no_endpoint += 1;
            continue;
        };
        let outcome =
            assert_port_routes_generic_analog(&board, &chip, &profile, port_index, &probe);
        silicon += usize::from(outcome.silicon_register);
        covered.push(board);
    }
    for (board, _) in INPUT_REGISTER_UNREACHED {
        assert!(
            covered.iter().any(|b| b == board),
            "{board} is listed as unreached but is not covered"
        );
    }
    assert_eq!(
        covered.len(),
        COVERED,
        "onboarding profiles covered through a GPIO input; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        silicon, SILICON_REGISTER,
        "profiles read back at a silicon input register; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        no_endpoint, NO_ANALOG_ENDPOINT,
        "onboarding profiles with no GPIO input"
    );
}
