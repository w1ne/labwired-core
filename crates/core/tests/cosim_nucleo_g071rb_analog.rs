// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive NUCLEO-G071RB GPIO inputs and explicit ADC1
//! channels. The ADC test proves routed conversion through the modeled L4
//! register interface; channel-to-pad aliases and silicon analog behavior are
//! not claimed.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_IOPENR: u64 = 0x4002_1034;
const RCC_APBENR2: u64 = 0x4002_1040;
const GPIOA_BASE: u64 = 0x5000_0000;
const GPIOA_MODER: u64 = GPIOA_BASE;
const GPIOA_IDR: u64 = GPIOA_BASE + 0x10;
const ADC1_BASE: u64 = 0x4001_2400;
const ADC1_ISR: u64 = ADC1_BASE;
const ADC1_CR: u64 = ADC1_BASE + 0x08;
const ADC1_SQR1: u64 = ADC1_BASE + 0x30;
const ADC1_DR: u64 = ADC1_BASE + 0x40;

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

/// Independent forward-active Gummel–Poon load line, with optional Early effect.
fn bjt_collector_voltage(base_drive_v: f64, early_voltage_v: Option<f64>) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-15;
    const BF: f64 = 100.0;
    const BR: f64 = 4.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let early_factor = early_voltage_v.map_or(1.0, |vaf| 1.0 - vbc / vaf);
        let transport = IS_A * (forward - reverse) * early_factor;
        let base_emitter = (IS_A / BF) * (forward - 1.0);
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = (low + high) / 2.0;
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = (SUPPLY_V - collector_v) / COLLECTOR_OHMS - collector_current;
            if residual > 0.0 {
                low = collector_v;
            } else {
                high = collector_v;
            }
        }
        (low + high) / 2.0
    };

    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let vbe = (low + high) / 2.0;
        let collector_v = collector_for_vbe(vbe);
        let (_, base_current) = currents(vbe, collector_v);
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe((low + high) / 2.0)
}

/// Independent Gummel–Poon load line with forward/reverse high-injection
/// rolloff and the same Early voltages in both branches.
fn bjt_high_injection_collector_voltage(base_drive_v: f64, high_injection: bool) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    const IKF_A: f64 = 50e-6;
    const IKR_A: f64 = 500e-6;
    const NKF: f64 = 0.8;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let q2 = if high_injection {
            IS_A * (forward - 1.0) / IKF_A + IS_A * (reverse - 1.0) / IKR_A
        } else {
            0.0
        };
        let qb = (1.0 + (1.0 + 4.0 * q2).max(0.0).powf(NKF)) / (2.0 * base_width);
        let transport = IS_A * (forward - reverse) / qb;
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let base_emitter = (IS_A / BF) * (forward - 1.0);
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = (low + high) / 2.0;
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = (SUPPLY_V - collector_v) / COLLECTOR_OHMS - collector_current;
            if residual > 0.0 {
                low = collector_v;
            } else {
                high = collector_v;
            }
        }
        (low + high) / 2.0
    };

    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let vbe = (low + high) / 2.0;
        let collector_v = collector_for_vbe(vbe);
        let (_, base_current) = currents(vbe, collector_v);
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe((low + high) / 2.0)
}

/// Independent Gummel–Poon load line with optional B-E and B-C recombination.
fn bjt_recombination_collector_voltage(
    base_drive_v: f64,
    base_emitter_recombination: bool,
    base_collector_recombination: bool,
) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    const ISE_A: f64 = 2e-12;
    const NE: f64 = 1.5;
    const ISC_A: f64 = 5e-10;
    const NC: f64 = 2.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let transport = IS_A * (forward - reverse) * base_width;
        let base_collector = (IS_A / BR) * (reverse - 1.0)
            + if base_collector_recombination {
                ISC_A * ((vbc / (NC * VT)).exp() - 1.0)
            } else {
                0.0
            };
        let base_emitter = (IS_A / BF) * (forward - 1.0)
            + if base_emitter_recombination {
                ISE_A * ((vbe / (NE * VT)).exp() - 1.0)
            } else {
                0.0
            };
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = 0.5 * (low + high);
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = (SUPPLY_V - collector_v) / COLLECTOR_OHMS - collector_current;
            if residual > 0.0 {
                low = collector_v;
            } else {
                high = collector_v;
            }
        }
        0.5 * (low + high)
    };

    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let vbe = 0.5 * (low + high);
        let collector_v = collector_for_vbe(vbe);
        let (_, base_current) = currents(vbe, collector_v);
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

/// Independent level-1 NJF load line with model-card drain/source resistances.
/// Gate-junction leakage is negligible at the fixture's IS=1e-30 A.
fn jfet_drain_voltage(gate_v: f64, drain_ohms: f64, source_ohms: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = -2.0;
    const BETA_A_PER_V2: f64 = 500e-6;
    const LAMBDA_PER_V: f64 = 0.025;

    let channel_current = |current_a: f64| {
        let external_drain_v = SUPPLY_V - current_a * LOAD_OHMS;
        let internal_drain_v = external_drain_v - current_a * drain_ohms;
        let internal_source_v = current_a * source_ohms;
        let vds = internal_drain_v - internal_source_v;
        let vgst = gate_v - internal_source_v - VTO;
        if vgst <= 0.0 {
            0.0
        } else if vds < vgst {
            BETA_A_PER_V2 * (1.0 + LAMBDA_PER_V * vds) * vds * (2.0 * vgst - vds)
        } else {
            BETA_A_PER_V2 * (1.0 + LAMBDA_PER_V * vds) * vgst * vgst
        }
    };

    // Solve I_D = I_channel(V_GS(I_D), V_DS(I_D)) independently from MNA.
    let (mut low, mut high) = (0.0, SUPPLY_V / LOAD_OHMS);
    for _ in 0..100 {
        let current_a = (low + high) / 2.0;
        if channel_current(current_a) > current_a {
            low = current_a;
        } else {
            high = current_a;
        }
    }
    SUPPLY_V - ((low + high) / 2.0) * LOAD_OHMS
}

/// Independent level-1 NMOS body-effect load line for ADC-routed G071 tests.
fn mosfet_body_effect_drain_voltage(gate_v: f64, bulk_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;
    const GAMMA: f64 = 0.4;
    const PHI: f64 = 0.7;

    let threshold = VTO + GAMMA * ((PHI - bulk_v).sqrt() - PHI.sqrt());
    let overdrive = gate_v - threshold;
    if overdrive <= 0.0 {
        return SUPPLY_V;
    }
    let beta = KP_A_PER_V2 * W_M / L_M;
    let drain_current = |vds: f64| {
        if vds < overdrive {
            beta * (1.0 + LAMBDA_PER_V * vds) * vds * (overdrive - 0.5 * vds)
        } else {
            0.5 * beta * (1.0 + LAMBDA_PER_V * vds) * overdrive * overdrive
        }
    };
    let residual = |vds: f64| SUPPLY_V - vds - LOAD_OHMS * drain_current(vds);
    let (mut low, mut high) = (0.0, SUPPLY_V);
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32G071 declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

/// Convert one explicit L4-family ADC channel through firmware-visible MMIO.
fn stm32g0_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const ADRDY: u32 = 1;
    const EOC: u32 = 1 << 2;
    const EOS: u32 = 1 << 3;
    const ADEN: u32 = 1;
    const ADSTART: u32 = 1 << 2;

    bus.write_u32(ADC1_SQR1, u32::from(channel) << 6)
        .expect("select ADC1 SQ1 channel");
    // ISR is write-one-to-clear. A low-byte write clears only the flags we
    // intend to clear and preserves ADRDY on this L4-family register block.
    bus.write_u8(ADC1_ISR, (EOC | EOS) as u8)
        .expect("clear prior conversion flags");
    let cr = bus.read_u32(ADC1_CR).expect("read ADC1 CR");
    assert_ne!(cr & ADEN, 0, "ADC1 is enabled");
    let isr = bus.read_u32(ADC1_ISR).unwrap();
    assert_ne!(
        isr & ADRDY,
        0,
        "ADC1 is ready after routed advance; ISR={isr:#010x}, CR={:#010x}",
        bus.read_u32(ADC1_CR).unwrap()
    );
    bus.write_u32(ADC1_CR, cr | ADSTART)
        .expect("start one regular conversion");
    let isr = bus.read_u32(ADC1_ISR).expect("read ADC1 ISR");
    assert_ne!(isr & EOC, 0, "conversion sets EOC");
    assert_ne!(isr & EOS, 0, "single-channel sequence sets EOS");
    bus.read_u32(ADC1_DR)
        .expect("read firmware-visible ADC1 DR")
        & 0x0FFF
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round();
    (millivolts * 4095.0 / 3300.0).floor().clamp(0.0, 4095.0) as u16
}

/// Independent PMOS Ward-Dutton drain charge from a Simpson integral of the
/// level-1 inversion charge along the channel.
fn pmos_ward_dutton_drain_charge(gate_v: f64, drain_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const WIDTH_M: f64 = 100e-6;
    const LENGTH_M: f64 = 10e-6;
    const TOX_M: f64 = 10e-9;
    const VTO_MAGNITUDE_V: f64 = 1.0;
    const EPSILON_OX_F_PER_M: f64 = 3.453_133e-11;
    const INTERVALS: usize = 4000;

    let overdrive = (SUPPLY_V - gate_v - VTO_MAGNITUDE_V).max(0.0);
    if overdrive == 0.0 {
        return 0.0;
    }
    let channel_drop = (SUPPLY_V - drain_v).clamp(0.0, overdrive);
    let denominator = overdrive * channel_drop - 0.5 * channel_drop * channel_drop;
    let mut integral = 0.0;
    for point in 0..=INTERVALS {
        let position = point as f64 / INTERVALS as f64;
        let local_charge = (overdrive * overdrive - 2.0 * position * denominator).sqrt();
        let weight = if point == 0 || point == INTERVALS {
            1.0
        } else if point % 2 == 0 {
            2.0
        } else {
            4.0
        };
        integral += weight * position * local_charge;
    }
    let cox = EPSILON_OX_F_PER_M * WIDTH_M * LENGTH_M / TOX_M;
    cox * integral / (3.0 * INTERVALS as f64)
}

/// Independent backward-Euler PMOS drain voltage, with the physical drain
/// tied to VDD through a load resistor and the source/bulk held at VDD.
fn pmos_ward_dutton_drain_voltage(
    gate_v: f64,
    previous_gate_v: f64,
    previous_drain_v: f64,
    resistance_ohms: f64,
    step_s: f64,
) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    let previous_charge = pmos_ward_dutton_drain_charge(previous_gate_v, previous_drain_v);
    let residual = |drain_v: f64| {
        (drain_v - SUPPLY_V) / resistance_ohms
            + (pmos_ward_dutton_drain_charge(gate_v, drain_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (0.0, SUPPLY_V);
    assert!(residual(low) <= 0.0 && residual(high) >= 0.0);
    for _ in 0..80 {
        let middle = 0.5 * (low + high);
        if residual(middle) < 0.0 {
            low = middle;
        } else {
            high = middle;
        }
    }
    0.5 * (low + high)
}

/// Independent backward-Euler base-node solve for a generic NPN with fixed
/// collector bias. With no Early effect or high-injection knees, the forward
/// transit charge is `Qbe = TF·IS·(e^(Vbe/Vt)-1)`.
fn bjt_transit_base_voltage(previous_v: f64, drive_v: f64, tf_s: f64, step_s: f64) -> f64 {
    const BASE_OHMS: f64 = 10_000.0;
    const COLLECTOR_V: f64 = 0.1;
    const IS_A: f64 = 1e-15;
    const BF: f64 = 100.0;
    const BR: f64 = 4.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let junction_current = |voltage_v: f64| IS_A * ((voltage_v / VT).exp() - 1.0);
    let charge = |vbe_v: f64| tf_s * junction_current(vbe_v);
    let previous_charge = charge(previous_v);
    let residual = |vbe_v: f64| {
        let vbc_v = vbe_v - COLLECTOR_V;
        (drive_v - vbe_v) / BASE_OHMS
            - junction_current(vbe_v) / BF
            - junction_current(vbc_v) / BR
            - (charge(vbe_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (0.0, drive_v);
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// Independent level-1 JFET gate-junction depletion charge below FC * PB.
fn jfet_gate_junction_charge(voltage_v: f64) -> f64 {
    const C0_F: f64 = 1e-9;
    const PB_V: f64 = 1.0;
    const M: f64 = 0.5;
    C0_F * PB_V / (1.0 - M) * (1.0 - (1.0 - voltage_v / PB_V).powf(1.0 - M))
}

/// Independent backward-Euler update for two equal gate junctions tied to
/// fixed 3.3 V drain/source nodes through their depletion charges.
fn expected_jfet_gate_voltage(previous_v: f64, drive_v: f64, step_s: f64) -> f64 {
    const DRIVE_OHMS: f64 = 1_000.0;
    const JUNCTION_REFERENCE_V: f64 = 3.3;
    let total_charge = |gate_v: f64| 2.0 * jfet_gate_junction_charge(gate_v - JUNCTION_REFERENCE_V);
    let previous_charge = total_charge(previous_v);
    let residual = |gate_v: f64| {
        (gate_v - drive_v) / DRIVE_OHMS + (total_charge(gate_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (drive_v.min(previous_v), drive_v.max(previous_v));
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            high = mid;
        } else {
            low = mid;
        }
    }
    0.5 * (low + high)
}

#[test]
fn nucleo_g071rb_ntc_and_diode_netlists_reach_pa0_pa1_gpio_inputs() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    assert_eq!(profile.name, "nucleo-g071rb");
    assert!(profile.external_devices.is_empty());
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "led_ld4" && io.peripheral == "gpioa" && io.pin == 5));
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "button_b1" && io.peripheral == "gpioc" && io.pin == 13));

    let analog =
        SystemManifest::from_file(root("examples/nucleo-g071rb/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    // Simulation assumptions only: no G071 threshold values are claimed here.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds::uniform(0.3, 0.7));

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA0"),
        Some((GPIOA_IDR, 0)),
        "PA0 must resolve to the modeled GPIOA IDR bit 0"
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA1"),
        Some((GPIOA_IDR, 1)),
        "PA1 must resolve to the modeled GPIOA IDR bit 1"
    );

    bus.write_u32(RCC_IOPENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xF)
        .expect("configure PA0/PA1 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to separate modeled PA0/PA1 GPIO inputs"
    );

    let expected_diode_v = diode_load_line_voltage();
    let vil_v = 0.3 * 3.3;
    assert!(expected_diode_v < vil_v, "diode must fall below test VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // hold high while the input sits inside the assumed band
        (80.0, false),
        (25.0, false), // hold low while it returns to the assumed band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(
            steps.len(),
            2,
            "both netlists cross the co-simulation boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioa_idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_eq!(
            gpioa_idr & 1 != 0,
            expected_ntc_level,
            "PA0 must follow the solved NTC voltage and retain threshold state"
        );
        assert_eq!(gpioa_idr & 2, 0, "PA1 must read low below the assumed VIL");
    }
}

#[test]
fn nucleo_g071rb_bjt_early_effect_reaches_pa2_pa3_gpio_inputs() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-g071rb/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC divider and Shockley diode");
    let bjt =
        SystemManifest::from_file(root("examples/nucleo-g071rb/analog-bjt-early-effect.yaml"))
            .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    // Simulation assumptions only: no G071 threshold values are claimed.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds::uniform(0.3, 0.7));

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");
    for (pad, bit) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&bus, pad),
            Some((GPIOA_IDR, bit)),
            "{pad} must resolve to its modeled GPIOA IDR bit"
        );
    }
    bus.write_u32(RCC_IOPENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xFF)
        .expect("configure PA0-PA3 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must route independently to PA0 through PA3"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .expect("set independent NTC source");
    session
        .set_signal_number("ui.bjt.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let vil_v = 0.3 * 3.3;
    let vih_v = 0.7 * 3.3;
    let mut early_high = false;
    let mut reference_high = false;
    let mut cycles = 0;
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the unrelated and matched device netlists");
        assert_eq!(steps.len(), 4, "all four analog branches solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        let early_v = signal_voltage(&session, "board.gpio_in.pa2");
        let reference_v = signal_voltage(&session, "board.gpio_in.pa3");
        let ntc_resistance = ntc_resistance_ohms(-10.0);
        let expected_ntc_v = 3.3 * ntc_resistance / (10_000.0 + ntc_resistance);
        let expected_diode_v = diode_load_line_voltage();
        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_collector_voltage(base_drive_v, None);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC voltage is stable"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated diode load line is stable"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 Gummel–Poon load line at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "no-Early-effect Gummel–Poon load line at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < reference_v - 0.005,
                "finite VAF increases forward collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
            );
        }

        if early_v >= vih_v {
            early_high = true;
        } else if early_v <= vil_v {
            early_high = false;
        }
        if reference_v >= vih_v {
            reference_high = true;
        } else if reference_v <= vil_v {
            reference_high = false;
        }
        let idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input levels");
        assert_ne!(idr & 1, 0, "the cold NTC remains above assumed VIH on PA0");
        assert_eq!(
            idr & 2,
            0,
            "the Shockley diode remains below assumed VIL on PA1"
        );
        assert_eq!(
            idr & 4 != 0,
            early_high,
            "PA2 follows the Early-effect collector voltage"
        );
        assert_eq!(
            idr & 8 != 0,
            reference_high,
            "PA3 follows the no-Early-effect collector voltage"
        );
    }
}

#[test]
fn nucleo_g071rb_ntc_and_diode_netlists_convert_through_adc1_channels() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load generic NTC and diode ADC-channel netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC and an unrelated diode");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models bind to separate explicit ADC1 channels"
    );
    for channel in [4, 5, 6] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve both netlists and route their ADC1 inputs");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.adc1.4_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let routed_diode_v = signal_voltage(&session, "adc.adc1.5_volts");
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the independent Shockley diode load line should be {diode_v} V, got {routed_diode_v} V"
        );

        let ntc_code = expected_adc_count(ntc_v);
        let diode_code = expected_adc_count(routed_diode_v);
        assert_eq!(adc_channel_count(&mut bus, 4), ntc_code, "NTC seeds IN4");
        assert_eq!(
            adc_channel_count(&mut bus, 5),
            diode_code,
            "diode seeds IN5"
        );
        assert_eq!(stm32g0_adc_convert(&mut bus, 4), u32::from(ntc_code));
        assert_eq!(stm32g0_adc_convert(&mut bus, 5), u32::from(diode_code));
        assert_eq!(
            adc_channel_count(&mut bus, 6),
            u16::MAX,
            "unrelated IN6 stays undriven"
        );
        assert_eq!(
            stm32g0_adc_convert(&mut bus, 6),
            3723,
            "undriven IN6 retains the L4-family modeled 3.0 V source"
        );
    }
}

#[test]
fn nucleo_g071rb_ntc_and_diode_pad_aliases_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-pad-adc-system.yaml",
    ))
    .expect("load generic NTC and diode routes through physical ADC pads");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC and an unrelated diode");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA4 and PA5 aliases bind to their G071 ADC1 channels"
    );
    for channel in [4, 5, 6] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve both netlists and route PA4/PA5 to ADC1");
        assert_eq!(steps.len(), 2, "both unrelated netlists solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa4_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "PA4 NTC voltage at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let routed_diode_v = signal_voltage(&session, "board.analog.pa5_volts");
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "PA5 diode load line should be {diode_v} V, got {routed_diode_v} V"
        );

        let ntc_code = expected_adc_count(ntc_v);
        let diode_code = expected_adc_count(routed_diode_v);
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            ntc_code,
            "PA4 resolves to IN4"
        );
        assert_eq!(
            adc_channel_count(&mut bus, 5),
            diode_code,
            "PA5 resolves to IN5"
        );
        assert_eq!(stm32g0_adc_convert(&mut bus, 4), u32::from(ntc_code));
        assert_eq!(stm32g0_adc_convert(&mut bus, 5), u32::from(diode_code));
        assert_eq!(
            adc_channel_count(&mut bus, 6),
            u16::MAX,
            "unrelated IN6 stays undriven"
        );
    }
}

#[test]
fn nucleo_g071rb_bjt_high_injection_and_diode_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load generic NTC divider and unrelated Shockley diode");
    let high_injection = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-bjt-high-injection-adc-system.yaml",
    ))
    .expect("load matched Gummel–Poon high-injection and control branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(high_injection.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models bind to independent ADC1 channels"
    );
    for channel in [4, 5, 6, 7] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");
    session
        .set_signal_number("ui.bjt.high_drive_v", 0.0)
        .expect("initialize high-injection BJT drive");
    session
        .set_signal_number("ui.bjt.control_drive_v", 0.0)
        .expect("initialize no-rolloff control drive");

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut rolloff_at_high_drive = None;
    let mut control_at_high_drive = None;
    for base_drive_v in [0.8_f64, 1.5, 3.3] {
        session
            .set_signal_number("ui.bjt.high_drive_v", base_drive_v)
            .expect("set high-injection BJT base drive");
        session
            .set_signal_number("ui.bjt.control_drive_v", base_drive_v)
            .expect("set matched control base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 3, "NTC, diode, and BJT models solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let high_v = bjt_high_injection_collector_voltage(base_drive_v, true);
        let control_v = bjt_high_injection_collector_voltage(base_drive_v, false);
        for (path, expected_v) in [
            ("adc.adc1.4_volts", ntc_v),
            ("adc.adc1.5_volts", diode_v),
            ("adc.adc1.6_volts", high_v),
            ("adc.adc1.7_volts", control_v),
        ] {
            let solved_v = signal_voltage(&session, path);
            assert!(
                (solved_v - expected_v).abs() < 5e-5,
                "{path} should resolve to {expected_v} V, got {solved_v} V"
            );
        }

        for (channel, voltage_v) in [(4, ntc_v), (5, diode_v), (6, high_v), (7, control_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved voltage seeds ADC1_IN{channel}"
            );
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion follows the solved voltage"
            );
        }
        for channel in [0, 1, 2, 3, 8] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                u16::MAX,
                "unrelated ADC1_IN{channel} remains isolated"
            );
        }

        if base_drive_v == 3.3 {
            rolloff_at_high_drive = Some(high_v);
            control_at_high_drive = Some(control_v);
        }
    }

    let rolloff_v = rolloff_at_high_drive.expect("high-drive rolloff sample was recorded");
    let control_v = control_at_high_drive.expect("high-drive control sample was recorded");
    assert!(
        rolloff_v - control_v > 0.05,
        "IKF/NKF high-injection rolloff lowers collector current and raises collector voltage: {rolloff_v} V vs {control_v} V"
    );
}

#[test]
fn nucleo_g071rb_mosfet_body_effect_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and diode ADC routes");
    let mosfet = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-mosfet-body-effect-adc-system.yaml",
    ))
    .expect("load matched generic NMOS body-effect branches");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched NMOS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic NTC, diode, and NMOS models bind to separate ADC1 channels"
    );
    for channel in 4..=7 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.mosfet_body.gate_v", 2.5)
        .expect("set shared generic NMOS gate drive");
    session
        .set_signal_number("ui.mosfet_body.bulk_v", 0.0)
        .expect("initialize NMOS bulk bias");

    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut body_samples = Vec::new();
    for bulk_v in [0.0_f64, -0.5, -1.0, -1.5] {
        session
            .set_signal_number("ui.mosfet_body.bulk_v", bulk_v)
            .expect("set NMOS reverse body bias");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched NMOS netlists");
        assert_eq!(steps.len(), 3, "NTC, diode, and NMOS pair models solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let expected_reference_v = mosfet_body_effect_drain_voltage(2.5, 0.0);
        let expected_body_v = mosfet_body_effect_drain_voltage(2.5, bulk_v);
        let expected_ntc_v = 1.65;
        for (path, expected_v) in [
            ("adc.adc1.4_volts", expected_ntc_v),
            ("adc.adc1.5_volts", diode_v),
            ("adc.adc1.6_volts", expected_reference_v),
            ("adc.adc1.7_volts", expected_body_v),
        ] {
            let solved_v = signal_voltage(&session, path);
            assert!(
                (solved_v - expected_v).abs() < 5e-5,
                "{path} should resolve to {expected_v} V, got {solved_v} V"
            );
        }
        let reference_v = signal_voltage(&session, "adc.adc1.6_volts");
        let body_v = signal_voltage(&session, "adc.adc1.7_volts");
        if bulk_v == 0.0 {
            assert!(
                (reference_v - body_v).abs() < 1e-9,
                "matched NMOS devices agree at zero body bias"
            );
        } else {
            assert!(
                body_v > reference_v,
                "reverse body bias raises Vth and the loaded drain voltage: {body_v} V vs {reference_v} V"
            );
        }

        for (channel, voltage_v) in [
            (4, expected_ntc_v),
            (5, diode_v),
            (6, reference_v),
            (7, body_v),
        ] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved voltage seeds ADC1_IN{channel}"
            );
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion follows the solved voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 8),
            u16::MAX,
            "neighboring ADC1_IN8 remains undriven"
        );
        body_samples.push((bulk_v, body_v));
    }
    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "increasing reverse body bias raises the drain voltage: {body_samples:?}"
    );
}

#[test]
fn nucleo_g071rb_jfet_terminal_resistances_reach_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let base = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode routes");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-jfet-terminal-resistance-adc-system.yaml",
    ))
    .expect("load matched generic level-1 JFET branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and four matched JFET branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models bind to independent ADC1 channels"
    );
    for channel in 4..=9 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");
    session
        .set_signal_number("ui.jfet.gate_v", -1.0)
        .expect("initialize matched JFET gate drive");

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let branch_channels = [
        ("control", 6, 0.0, 0.0),
        ("drain", 7, 200.0, 0.0),
        ("source", 8, 0.0, 200.0),
        ("both", 9, 200.0, 200.0),
    ];
    let mut cycles = 0;
    let mut settled_jfet_voltages = None;
    for gate_v in [-1.0_f64, 0.0] {
        session
            .set_signal_number("ui.jfet.gate_v", gate_v)
            .expect("set matched JFET gate drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and JFET branches");
        assert_eq!(steps.len(), 3, "NTC, diode, and JFET netlists solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_solved_v = signal_voltage(&session, "adc.adc1.4_volts");
        let diode_solved_v = signal_voltage(&session, "adc.adc1.5_volts");
        assert!(
            (ntc_solved_v - ntc_v).abs() < 1e-9,
            "unrelated NTC remains isolated"
        );
        assert!(
            (diode_solved_v - diode_v).abs() < 2e-5,
            "unrelated diode remains on its independent load line"
        );

        let mut jfet_voltages = Vec::new();
        for (name, channel, rd, rs) in branch_channels {
            let path = format!("adc.adc1.{channel}_volts");
            let solved_v = signal_voltage(&session, &path);
            let expected_v = jfet_drain_voltage(gate_v, rd, rs);
            assert!(
                (solved_v - expected_v).abs() < 5e-5,
                "{name} NJF load line at gate={gate_v} V should be {expected_v} V, got {solved_v} V"
            );
            let expected = expected_adc_count(solved_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved {name} JFET voltage seeds ADC1_IN{channel}"
            );
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion follows the {name} JFET"
            );
            jfet_voltages.push(solved_v);
        }

        for (channel, voltage_v) in [(4, ntc_solved_v), (5, diode_solved_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(adc_channel_count(&mut bus, channel), expected);
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion preserves the unrelated device"
            );
        }
        for channel in [0, 1, 2, 3, 10] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                u16::MAX,
                "unrelated ADC1_IN{channel} remains isolated"
            );
        }
        settled_jfet_voltages = Some(jfet_voltages);
    }

    let [control, drain, source, both] = settled_jfet_voltages
        .expect("the final JFET gate-drive sample was recorded")
        .try_into()
        .expect("all four JFET branches were routed");
    assert!(
        drain > control + 0.05,
        "drain resistance lowers channel current and raises external drain voltage: {drain} V vs {control} V"
    );
    assert!(
        source > control + 0.1,
        "source resistance raises the intrinsic source and reduces channel current: {source} V vs {control} V"
    );
    assert!(
        both > drain && both > source,
        "combined RD and RS increase the external drain voltage beyond either terminal alone: {both} V, {drain} V, {source} V"
    );
}

#[test]
fn nucleo_g071rb_jfet_gate_depletion_charge_reaches_adc1_alongside_ntc_and_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let base = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-jfet-gate-charge-adc-system.yaml",
    ))
    .expect("load matched generic JFET gate-charge branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");
    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and two matched JFET branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models bind to separate ADC1 channels 4 through 7"
    );
    for channel in 4..=8 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.jfet_gate.drive_v", 3.3)
        .expect("precharge both JFET gates at their drain/source reference");
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated NTC temperature");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC, diode, and both JFET branches");
    assert_eq!(initial_steps.len(), 4, "all four analog netlists solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        (signal_voltage(&session, "adc.adc1.6_volts") - 3.3).abs() < 1e-9,
        "the precharged JFET gate starts at the drain/source reference"
    );

    session
        .set_signal_number("ui.jfet_gate.drive_v", 0.0)
        .expect("step both JFET gate drives low");
    session
        .set_signal_number("ui.ntc.temperature_c", 80.0)
        .expect("heat the unrelated NTC while the JFET gates discharge");
    let ntc_resistance = ntc_resistance_ohms(80.0);
    let expected_ntc_v = 3.3 * ntc_resistance / (10_000.0 + ntc_resistance);
    let mut expected_gate_v = 3.3;
    for step in 0..8 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route JFET charge alongside NTC and diode");
        assert_eq!(steps.len(), 4, "all netlists solve at every step");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        for _ in 0..SUBSTEPS {
            expected_gate_v =
                expected_jfet_gate_voltage(expected_gate_v, 0.0, STEP_S / SUBSTEPS as f64);
        }
        let gate_v = signal_voltage(&session, "adc.adc1.6_volts");
        let control_v = signal_voltage(&session, "adc.adc1.7_volts");
        assert!(
            (gate_v - expected_gate_v).abs() < 2e-8,
            "JFET CGS/CGD depletion charge at step {step}: expected {expected_gate_v} V, got {gate_v} V"
        );
        assert!(
            control_v.abs() < 1e-8,
            "the zero-charge reference follows the low drive within the model's leakage floor, got {control_v} V"
        );
        assert!(
            gate_v > control_v,
            "depletion charge must delay the gate edge relative to its zero-charge control"
        );

        let ntc_v = signal_voltage(&session, "adc.adc1.4_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC remains at its 80 °C divider voltage"
        );
        let diode_v = signal_voltage(&session, "adc.adc1.5_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains at its DC load line"
        );

        for (channel, voltage_v) in [(4, ntc_v), (5, diode_v), (6, gate_v), (7, control_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 IN{channel}"
            );
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 IN{channel} conversion reports the routed voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 8),
            u16::MAX,
            "neighboring ADC1 IN8 stays undriven"
        );
    }
    assert!(
        signal_voltage(&session, "adc.adc1.6_volts") < 1.0,
        "the gate depletion capacitance delays the edge but eventually follows the low drive"
    );
}

#[test]
fn nucleo_g071rb_pmos_tox_gate_charge_reaches_adc1_alongside_ntc_and_diode() {
    const DRIVE_OHMS: f64 = 1e6;
    const WIDTH_M: f64 = 100e-6;
    const LENGTH_M: f64 = 10e-6;
    const TOX_M: f64 = 10e-9;
    const EPSILON_OX_F_PER_M: f64 = 3.453_133e-11;
    const STEP_S: f64 = 1e-6;
    const GATE_DRIVE_V: f64 = 3.1;

    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let unrelated = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode routes");
    let pmos = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-mosfet-pmos-tox-charge-adc-system.yaml",
    ))
    .expect("load matched generic PMOS TOX charge branches");
    profile.cosim_models = unrelated.cosim_models;
    profile.cosim_models.extend(pmos.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");
    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched PMOS charge branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models bind to ADC1 channels 4 through 7"
    );
    for channel in 4..=8 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.pmos_tox.gate_v", 3.3)
        .expect("precharge the PMOS gate at its source and bulk");
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated NTC temperature");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the NTC, diode and PMOS gate branches");
    assert_eq!(initial_steps.len(), 3, "all three analog models solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        (signal_voltage(&session, "adc.adc1.6_volts") - 3.3).abs() < 1e-9
            && (signal_voltage(&session, "adc.adc1.7_volts") - 3.3).abs() < 1e-9,
        "the precharged PMOS gate and zero-charge control begin at the 3.3 V source"
    );

    session
        .set_signal_number("ui.pmos_tox.gate_v", GATE_DRIVE_V)
        .expect("step the matched PMOS gate branches below threshold");
    session
        .set_signal_number("ui.ntc.temperature_c", 80.0)
        .expect("heat the unrelated Beta NTC while the PMOS gate charges");
    let hot_ntc_resistance = ntc_resistance_ohms(80.0);
    let expected_ntc_v = 3.3 * hot_ntc_resistance / (10_000.0 + hot_ntc_resistance);
    let expected_diode_v = diode_load_line_voltage();
    let cox_f = EPSILON_OX_F_PER_M * WIDTH_M * LENGTH_M / TOX_M;
    let mut expected_gate_v = 3.3;

    for step in 0..24 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route PMOS TOX charge alongside NTC and diode");
        assert_eq!(
            steps.len(),
            3,
            "all three analog models solve at every step"
        );
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        expected_gate_v = (DRIVE_OHMS * cox_f * expected_gate_v + STEP_S * GATE_DRIVE_V)
            / (DRIVE_OHMS * cox_f + STEP_S);
        let ntc_v = signal_voltage(&session, "adc.adc1.4_volts");
        let diode_v = signal_voltage(&session, "adc.adc1.5_volts");
        let gate_v = signal_voltage(&session, "adc.adc1.6_volts");
        let reference_v = signal_voltage(&session, "adc.adc1.7_volts");

        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC follows its independent 80 °C divider"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays at its DC load line"
        );
        assert!(
            (reference_v - GATE_DRIVE_V).abs() < 1e-9,
            "the no-TOX PMOS gate follows its drive immediately"
        );
        assert!(
            (gate_v - expected_gate_v).abs() < 2e-7,
            "independent PMOS Meyer TOX RC recurrence at step {step}: expected {expected_gate_v} V, got {gate_v} V"
        );
        assert!(
            gate_v > reference_v,
            "PMOS oxide charge must delay the falling gate edge"
        );

        for (channel, voltage_v) in [(4, ntc_v), (5, diode_v), (6, gate_v), (7, reference_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 channel {channel} conversion reports {voltage_v} V"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 8),
            u16::MAX,
            "neighboring ADC1_IN8 remains undriven"
        );
    }

    assert!(
        (expected_gate_v - GATE_DRIVE_V).abs() < 1e-3,
        "PMOS gate voltage approaches its DC drive after repeated steps"
    );
}

#[test]
fn nucleo_g071rb_pmos_ward_dutton_channel_charge_reaches_adc1_with_ntc_and_diode() {
    const SUPPLY_V: f64 = 3.3;
    const GATE_ON_V: f64 = 1.8;
    const LOAD_OHMS: f64 = 5e6;
    const STEP_S: f64 = 1e-6;

    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let unrelated = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode routes");
    let pmos = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-mosfet-pmos-ward-dutton-adc-system.yaml",
    ))
    .expect("load matched generic PMOS channel-charge branches");
    profile.cosim_models = unrelated.cosim_models;
    profile.cosim_models.extend(pmos.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");
    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and two PMOS channel-charge branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models bind to ADC1 channels 4 through 7"
    );
    for channel in 4..=8 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.pmos_ward_dutton.gate_v", SUPPLY_V)
        .expect("initialize PMOS gates at their source and bulk");
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the NTC, diode, and PMOS drain branches");
    assert_eq!(initial_steps.len(), 3, "all three analog models solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        (signal_voltage(&session, "adc.adc1.6_volts") - SUPPLY_V).abs() < 1e-9
            && (signal_voltage(&session, "adc.adc1.7_volts") - SUPPLY_V).abs() < 1e-9,
        "both PMOS drain nodes start at the source voltage"
    );

    session
        .set_signal_number("ui.pmos_ward_dutton.gate_v", GATE_ON_V)
        .expect("turn on the matched PMOS branches");
    session
        .set_signal_number("ui.ntc.temperature_c", 80.0)
        .expect("heat the unrelated Beta NTC while the PMOS channel charges");
    let hot_ntc_resistance = ntc_resistance_ohms(80.0);
    let expected_ntc_v = 3.3 * hot_ntc_resistance / (10_000.0 + hot_ntc_resistance);
    let expected_diode_v = diode_load_line_voltage();
    let mut previous_gate_v = SUPPLY_V;
    let mut previous_drain_v = SUPPLY_V;

    for step in 0..8 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route PMOS channel charge beside NTC and diode");
        assert_eq!(
            steps.len(),
            3,
            "all three analog models solve at every step"
        );
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let expected_drain_v = pmos_ward_dutton_drain_voltage(
            GATE_ON_V,
            previous_gate_v,
            previous_drain_v,
            LOAD_OHMS,
            STEP_S,
        );
        let ntc_v = signal_voltage(&session, "adc.adc1.4_volts");
        let diode_v = signal_voltage(&session, "adc.adc1.5_volts");
        let charged_v = signal_voltage(&session, "adc.adc1.6_volts");
        let reference_v = signal_voltage(&session, "adc.adc1.7_volts");

        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated diode stays at its load line"
        );
        assert!(
            (charged_v - expected_drain_v).abs() < 5e-5,
            "independent PMOS Ward-Dutton charge recurrence at step {step}: expected {expected_drain_v} V, got {charged_v} V"
        );
        assert!(
            (reference_v - SUPPLY_V).abs() < 1e-9,
            "the matched no-channel-charge PMOS stays at its zero-current load line"
        );

        for (channel, voltage_v) in [(4, ntc_v), (5, diode_v), (6, charged_v), (7, reference_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 channel {channel} conversion reports {voltage_v} V"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 8),
            u16::MAX,
            "neighboring ADC1_IN8 remains undriven"
        );

        if step == 0 {
            assert!(
                charged_v < SUPPLY_V - 0.5,
                "PMOS inversion charge must move the drain measurably"
            );
            assert!(
                (charged_v - reference_v).abs() > 0.5,
                "the zero-charge control must distinguish Ward-Dutton inversion charge"
            );
        }
        previous_gate_v = GATE_ON_V;
        previous_drain_v = charged_v;
    }

    assert!(
        (previous_drain_v - SUPPLY_V).abs() < 0.2,
        "PMOS channel charge must settle back toward the DC load line"
    );
}

#[test]
fn nucleo_g071rb_diode_depletion_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let charge = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-diode-charge-adc-system.yaml",
    ))
    .expect("load generic diode depletion/transit charge comparisons");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and four diode charge variants");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models bind to independent ADC1 channels"
    );
    for channel in 4..=13 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.diode_charge.supply_v", 0.0)
        .expect("initialize diode charge branches at zero volts");

    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC and diode charge branches before the supply step");
    assert_eq!(initial_steps.len(), 6, "all six analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected initial ADC route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.diode_charge.supply_v", 3.3)
        .expect("apply a supply step to the four diode branches");
    let mut samples = Vec::new();
    for step in 0..800 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC and all four diode branches");
        assert_eq!(steps.len(), 6, "all six analog branches solve at each step");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.adc1.4_volts");
        let unrelated_diode_v = signal_voltage(&session, "adc.adc1.5_volts");
        let reference_v = signal_voltage(&session, "adc.adc1.10_volts");
        let depletion_v = signal_voltage(&session, "adc.adc1.11_volts");
        let transit_v = signal_voltage(&session, "adc.adc1.12_volts");
        let combined_v = signal_voltage(&session, "adc.adc1.13_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated 25 °C NTC stays isolated on ADC1_IN4"
        );
        assert!(
            (unrelated_diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated DC diode stays isolated on ADC1_IN5"
        );
        assert!(
            (reference_v - diode_load_line_voltage()).abs() < 2e-5,
            "uncharged reference should sit at the DC load line {} V, got {reference_v} V at step {step}",
            diode_load_line_voltage()
        );

        for (channel, volts) in [
            (4, ntc_v),
            (5, unrelated_diode_v),
            (10, reference_v),
            (11, depletion_v),
            (12, transit_v),
            (13, combined_v),
        ] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion must report the voltage on channel {channel}"
            );
        }
        for channel in [6, 7, 8, 9, 14] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                u16::MAX,
                "unrelated ADC1_IN{channel} remains undriven"
            );
        }

        samples.push((reference_v, depletion_v, transit_v, combined_v));
        if step == 0 {
            assert!(
                (depletion_v - reference_v).abs() > 0.001,
                "CJO depletion charge must change the first routed transient: {samples:?}"
            );
            assert!(
                (transit_v - reference_v).abs() > 0.01,
                "TT diffusion charge must change the first routed transient: {samples:?}"
            );
            assert!(
                (combined_v - reference_v).abs() > 0.01,
                "combined junction and transit charge must change the first routed transient: {samples:?}"
            );
        }
    }

    let settled = samples.last().expect("transient samples exist");
    for (label, voltage) in [
        ("depletion", settled.1),
        ("transit", settled.2),
        ("combined", settled.3),
    ] {
        assert!(
            (voltage - settled.0).abs() < 2e-4,
            "{label} charge branch must settle to the same DC diode load line, got {voltage} V vs {} V",
            settled.0
        );
    }
}

#[test]
fn nucleo_g071rb_bjt_base_recombination_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let recombination = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-bjt-recombination-adc-system.yaml",
    ))
    .expect("load matched generic BJT recombination netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(recombination.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and four matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices bind to independent ADC1 channels"
    );
    for channel in [8, 9, 10, 11] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without a BJT source"
        );
    }
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_recombination.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");

    let mut cycles = 0;
    let mut saw_base_emitter_effect = false;
    let mut saw_base_collector_effect = false;
    for base_drive_v in [0.7_f64, 0.9, 1.2] {
        session
            .set_signal_number("ui.bjt_recombination.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and four BJT branches");
        assert_eq!(steps.len(), 6, "all six generic analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.adc1.4_volts");
        let diode_v = signal_voltage(&session, "adc.adc1.5_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );

        let bjt_voltages = [
            signal_voltage(&session, "adc.adc1.8_volts"),
            signal_voltage(&session, "adc.adc1.9_volts"),
            signal_voltage(&session, "adc.adc1.10_volts"),
            signal_voltage(&session, "adc.adc1.11_volts"),
        ];
        let expected = [
            bjt_recombination_collector_voltage(base_drive_v, false, false),
            bjt_recombination_collector_voltage(base_drive_v, true, false),
            bjt_recombination_collector_voltage(base_drive_v, false, true),
            bjt_recombination_collector_voltage(base_drive_v, true, true),
        ];
        for (index, (voltage, expected_v)) in bjt_voltages.iter().zip(expected).enumerate() {
            assert!(
                (voltage - expected_v).abs() < 5e-5,
                "ADC1 BJT branch {index} at base drive {base_drive_v} V should match independent load line {expected_v} V, got {voltage} V"
            );
        }
        assert!(
            bjt_voltages[1] > bjt_voltages[0] + 0.005,
            "ISE/NE base-emitter recombination loads the base drive and raises collector voltage"
        );
        saw_base_emitter_effect |= bjt_voltages[1] - bjt_voltages[0] > 0.005;
        saw_base_collector_effect |= bjt_voltages[2] - bjt_voltages[0] > 0.1;

        for (channel, voltage_v) in [(4, ntc_v), (5, diode_v)]
            .into_iter()
            .chain((8..=11).zip(bjt_voltages))
        {
            let expected_count = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                stm32g0_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "modeled ADC1 conversion follows the generic device on channel {channel}"
            );
        }
        for channel in [6, 7, 12, 13] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                u16::MAX,
                "unrelated ADC1_IN{channel} remains undriven"
            );
        }
    }
    assert!(
        saw_base_emitter_effect,
        "ISE/NE effect is resolved at the ADC"
    );
    assert!(
        saw_base_collector_effect,
        "forward-biased ISC/NC recombination changes the collector load line"
    );
}

#[test]
fn nucleo_g071rb_bjt_terminal_resistances_reach_adc1_with_explicit_network_reference() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let terminal_resistances = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-bjt-terminal-resistance-adc-system.yaml",
    ))
    .expect("load matched generic BJT terminal-resistance netlists");
    profile.cosim_models = generic.cosim_models;
    profile
        .cosim_models
        .extend(terminal_resistances.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC, diode, and six matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices bind to independent ADC1 channels"
    );
    for channel in [8, 9, 10, 11, 12, 13] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} begins without a BJT source"
        );
    }
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_terminal.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");

    let mut cycles = 0;
    let mut settled = None;
    for base_drive_v in [0.7_f64, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt_terminal.base_v", base_drive_v)
            .expect("step all matched BJT base drives");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT terminal-resistance branches");
        assert_eq!(steps.len(), 8, "all eight generic analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.adc1.4_volts");
        let diode_v = signal_voltage(&session, "adc.adc1.5_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );

        let bjt_voltages = [
            signal_voltage(&session, "adc.adc1.8_volts"),
            signal_voltage(&session, "adc.adc1.9_volts"),
            signal_voltage(&session, "adc.adc1.10_volts"),
            signal_voltage(&session, "adc.adc1.11_volts"),
            signal_voltage(&session, "adc.adc1.12_volts"),
            signal_voltage(&session, "adc.adc1.13_volts"),
        ];
        assert!(
            (bjt_voltages[0] - bjt_collector_voltage(base_drive_v, Some(50.0))).abs() < 5e-5,
            "zero-resistance Gummel–Poon branch matches the independent load line"
        );
        assert!(
            (bjt_voltages[4] - bjt_voltages[5]).abs() < 1e-7,
            "model-card RB/RC/RE matches the equivalent explicit resistor network: {} V vs {} V",
            bjt_voltages[4],
            bjt_voltages[5]
        );

        if base_drive_v == 0.9 {
            for (label, voltage) in [
                ("RB", bjt_voltages[1]),
                ("RC", bjt_voltages[2]),
                ("RE", bjt_voltages[3]),
                ("RB+RC+RE", bjt_voltages[4]),
            ] {
                assert!(
                    (voltage - bjt_voltages[0]).abs() > 1e-3,
                    "{label} must change the collector load line: no-R={} V, actual={voltage} V",
                    bjt_voltages[0]
                );
            }
        }
        settled = Some((ntc_v, diode_v, bjt_voltages));
    }

    let (ntc_v, diode_v, bjt_voltages) = settled.expect("final base-drive sample exists");
    for (channel, voltage_v) in [(4, ntc_v), (5, diode_v)]
        .into_iter()
        .chain((8..=13).zip(bjt_voltages))
    {
        let expected_count = expected_adc_count(voltage_v);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected_count,
            "solved voltage seeds ADC1 channel {channel}"
        );
        assert_eq!(
            stm32g0_adc_convert(&mut bus, channel),
            u32::from(expected_count),
            "modeled ADC1 conversion follows the generic device on channel {channel}"
        );
    }
    for channel in [6, 7, 14, 15] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "unrelated ADC1_IN{channel} remains undriven"
        );
    }
}

#[test]
fn nucleo_g071rb_bjt_transit_charge_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC divider and Shockley diode");
    let transit = SystemManifest::from_file(root(
        "examples/nucleo-g071rb/analog-bjt-transit-adc-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon transit-charge branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(transit.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");

    const ADCEN: u32 = 1 << 20;
    bus.write_u32(RCC_APBENR2, ADCEN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_CR, 0)
        .expect("leave ADC deep power-down");
    bus.write_u32(ADC1_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC1_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC1_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC, diode, and matched BJT transit branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices bind to independent ADC1 channels"
    );
    for channel in [4, 5, 8, 9] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_transit.base_v", 0.0)
        .expect("initialize both BJT base-drive branches");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize NTC, diode, and BJT charge state");
    assert_eq!(initial_steps.len(), 4, "all four analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected ADC route errors: {initial_errors:?}"
    );

    const BASE_DRIVE_V: f64 = 0.8;
    const STEP_S: f64 = 1e-6;
    const TRANSIT_S: f64 = 1e-6;
    session
        .set_signal_number("ui.bjt_transit.base_v", BASE_DRIVE_V)
        .expect("step both matched BJT base drives");
    let mut previous_transit_v = 0.0;
    let mut first_difference = None;
    let mut final_voltages = None;
    for step in 0..1_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT transit charge");
        assert_eq!(steps.len(), 4, "all four generic branches solve each step");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.adc1.4_volts");
        let diode_v = signal_voltage(&session, "adc.adc1.5_volts");
        let control_v = signal_voltage(&session, "adc.adc1.8_volts");
        let transit_v = signal_voltage(&session, "adc.adc1.9_volts");
        let expected_control_v = bjt_transit_base_voltage(0.0, BASE_DRIVE_V, 0.0, STEP_S);
        let expected_transit_v =
            bjt_transit_base_voltage(previous_transit_v, BASE_DRIVE_V, TRANSIT_S, STEP_S);
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "zero-TF BJT must match the independent DC base solution {expected_control_v} V, got {control_v} V"
        );
        assert!(
            (transit_v - expected_transit_v).abs() < 5e-3,
            "TF backward-Euler base solution at step {step} should be {expected_transit_v} V, got {transit_v} V"
        );

        for (channel, voltage_v) in [(4, ntc_v), (5, diode_v), (8, control_v), (9, transit_v)] {
            let expected_count = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "solved voltage seeds ADC1 channel {channel}"
            );
            if step == 0 || step == 999 {
                assert_eq!(
                    stm32g0_adc_convert(&mut bus, channel),
                    u32::from(expected_count),
                    "firmware-visible ADC1 conversion follows channel {channel}"
                );
            }
        }
        for channel in [6, 7, 10, 11] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                u16::MAX,
                "unrelated ADC1_IN{channel} remains undriven"
            );
        }

        if step == 0 {
            first_difference = Some(control_v - transit_v);
        }
        previous_transit_v = expected_transit_v;
        final_voltages = Some((control_v, transit_v));
    }

    let (control_v, transit_v) = final_voltages.expect("final transient sample exists");
    assert!(
        first_difference.unwrap_or_default() > 0.05,
        "TF must store forward charge and lower the first-step base voltage"
    );
    assert!(
        (control_v - transit_v).abs() < 2e-4,
        "transit charge changes the transient only; both branches settle to the same DC base voltage"
    );
}
