// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the nRF5340-DK's application-core SAADC inputs.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const SAADC: u64 = 0x5000_E000;
const RESULT: u64 = 0x2000_0000;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const SERIES_LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / SERIES_LIMIT_OHMS - current
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

fn voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn expected_saadc_code(volts: f64) -> u16 {
    let millivolts = (volts * 1000.0).round() as u32;
    ((millivolts * 4096 / 3300).min(4095)) as u16
}

fn configure_saadc(bus: &mut SystemBus) {
    configure_saadc_channels(bus, 3);
}

fn configure_saadc_channels(bus: &mut SystemBus, channel_count: u32) {
    // Channel k selects AINk. Tests reserve extra channels as undriven controls.
    for channel in 0..channel_count {
        let base = SAADC + 0x510 + u64::from(channel) * 0x10;
        bus.write_u32(base, channel + 1)
            .expect("select SAADC positive input");
        bus.write_u32(base + 8, 0x1200)
            .expect("select gain and VDD/4 reference");
    }
    bus.write_u32(SAADC + 0x5f0, 2)
        .expect("select 12-bit SAADC resolution");
    bus.write_u32(SAADC + 0x62c, RESULT as u32)
        .expect("set SAADC EasyDMA result pointer");
    bus.write_u32(SAADC + 0x630, channel_count)
        .expect("reserve channel results");
    bus.write_u32(SAADC + 0x500, 1).expect("enable SAADC");
}

fn scan(bus: &mut SystemBus) -> [u16; 3] {
    scan_saadc_channels::<3>(bus)
}

fn scan_saadc_channels<const N: usize>(bus: &mut SystemBus) -> [u16; N] {
    bus.write_u32(SAADC + 0x104, 0)
        .expect("clear prior END event");
    bus.write_u32(SAADC, 1).expect("arm SAADC EasyDMA scan");
    bus.write_u32(SAADC + 4, 1)
        .expect("sample configured SAADC channels");
    bus.tick_peripherals_fully();

    assert_eq!(bus.read_u32(SAADC + 0x104).unwrap(), 1, "scan sets END");
    assert_eq!(
        bus.read_u32(SAADC + 0x634).unwrap(),
        N as u32,
        "scan writes every reserved result"
    );
    std::array::from_fn(|channel| bus.read_u16(RESULT + channel as u64 * 2).unwrap())
}

/// Independent Gummel–Poon NPN load-line reference including forward/reverse
/// Early effect and optional high-injection rolloff.
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

/// Independent ngspice level-1 base-spreading resistance interpolation.
fn bjt_base_resistance_ohms(base_current_a: f64, rb_ohms: f64, rbm_ohms: f64, irb_a: f64) -> f64 {
    let ratio = (base_current_a / irb_a).max(1e-9);
    let z = ((1.0 + 14.59025 * ratio).sqrt() - 1.0) / (2.4317 * ratio.sqrt());
    let tan_z = z.tan();
    rbm_ohms + 3.0 * (rb_ohms - rbm_ohms) * (tan_z - z) / (z * tan_z * tan_z)
}

/// Independent Gummel–Poon collector load line with explicit temperature-
/// adjusted RBM/IRB values. TLEV=3 with zero IS coefficients in the fixture
/// keeps the saturation current and beta fixed; only thermal voltage and the
/// base resistance coefficients change with instance temperature.
fn bjt_base_resistance_temperature_collector_voltage(
    base_drive_v: f64,
    rbm_ohms: f64,
    irb_a: f64,
    temperature_c: f64,
) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const DRIVE_OHMS: f64 = 2_000.0;
    const RB_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    let thermal_v = 1.380_649e-23 * (temperature_c + 273.15) / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / thermal_v).exp();
        let reverse = (vbc / thermal_v).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let transport = IS_A * (forward - reverse) * base_width;
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let base_emitter = (IS_A / BF) * (forward - 1.0);
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
        let base_resistance = bjt_base_resistance_ohms(base_current, RB_OHMS, rbm_ohms, irb_a);
        let residual = (base_drive_v - vbe) / (DRIVE_OHMS + base_resistance) - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

/// Independent hot Gummel–Poon reverse-active load line. The collector is
/// pulled to ground through `RCOL`; positive base-collector bias makes its
/// recombination current visible at the collector endpoint.
fn bjt_recombination_temperature_collector_voltage(
    base_drive_v: f64,
    ise_a: f64,
    isc_a: f64,
    temperature_c: f64,
) -> f64 {
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 2_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    const NE: f64 = 1.5;
    const NC: f64 = 2.0;
    let thermal_v = 1.380_649e-23 * (temperature_c + 273.15) / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / thermal_v).exp();
        let reverse = (vbc / thermal_v).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let transport = IS_A * (forward - reverse) * base_width;
        let base_collector =
            (IS_A / BR) * (reverse - 1.0) + isc_a * ((vbc / (NC * thermal_v)).exp() - 1.0);
        let base_emitter =
            (IS_A / BF) * (forward - 1.0) + ise_a * ((vbe / (NE * thermal_v)).exp() - 1.0);
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, base_drive_v);
        for _ in 0..100 {
            let collector_v = 0.5 * (low + high);
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = collector_v / COLLECTOR_OHMS + collector_current;
            if residual < 0.0 {
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

/// Independent hot Gummel–Poon load lines with zero Early voltage terms and
/// zero high-injection terms. The first result is a common-emitter collector;
/// the second is the high-side emitter in reverse-active operation.
fn bjt_beta_temperature_voltage(base_drive_v: f64, bf: f64, br: f64, reverse_active: bool) -> f64 {
    bjt_hot_load_line_voltage(base_drive_v, 1e-16, bf, br, reverse_active)
}

/// Independent hot Gummel–Poon load line for arbitrary transport current and
/// forward/reverse gains, with Early effect and high-injection terms disabled.
fn bjt_hot_load_line_voltage(
    base_drive_v: f64,
    is_a: f64,
    bf: f64,
    br: f64,
    reverse_active: bool,
) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const TEMP_K: f64 = 125.0 + 273.15;
    const BASE_THERMAL_V: f64 = 1.380_649e-23 * TEMP_K / 1.602_176_634e-19;
    let currents = |vbe: f64, vbc: f64| {
        let forward = (vbe / BASE_THERMAL_V).exp();
        let reverse = (vbc / BASE_THERMAL_V).exp();
        let transport = is_a * (forward - reverse);
        let base_emitter = (is_a / bf) * (forward - 1.0);
        let base_collector = (is_a / br) * (reverse - 1.0);
        let collector = transport - base_collector;
        let base = base_emitter + base_collector;
        (collector, base, -(collector + base))
    };

    if !reverse_active {
        const LOAD_OHMS: f64 = 10_000.0;
        const BASE_OHMS: f64 = 1_000_000.0;
        let collector_for_vbe = |vbe: f64| {
            let (mut low, mut high) = (0.0, SUPPLY_V);
            for _ in 0..100 {
                let collector_v = 0.5 * (low + high);
                let (collector_current, _, _) = currents(vbe, vbe - collector_v);
                let residual = (SUPPLY_V - collector_v) / LOAD_OHMS - collector_current;
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
            let (_, base_current, _) = currents(vbe, vbe - collector_v);
            let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
            if residual > 0.0 {
                low = vbe;
            } else {
                high = vbe;
            }
        }
        return collector_for_vbe(0.5 * (low + high));
    }

    const LOAD_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 2_000.0;
    let emitter_for_vbase = |base_v: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let emitter_v = 0.5 * (low + high);
            let (_, _, emitter_current) = currents(base_v - emitter_v, base_v);
            let residual = (SUPPLY_V - emitter_v) / LOAD_OHMS - emitter_current;
            if residual > 0.0 {
                low = emitter_v;
            } else {
                high = emitter_v;
            }
        }
        0.5 * (low + high)
    };
    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let base_v = 0.5 * (low + high);
        let emitter_v = emitter_for_vbase(base_v);
        let (_, base_current, _) = currents(base_v - emitter_v, base_v);
        let residual = (base_drive_v - base_v) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = base_v;
        } else {
            high = base_v;
        }
    }
    emitter_for_vbase(0.5 * (low + high))
}

/// Independent hot Gummel–Poon load lines with the square-root high-injection
/// base charge. `IKF` is exercised in common-emitter operation and `IKR` in
/// reverse-active operation, with Early effect disabled in the fixture.
fn bjt_knee_temperature_voltage(
    base_drive_v: f64,
    ikf_a: f64,
    ikr_a: f64,
    reverse_active: bool,
) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const TEMP_K: f64 = 125.0 + 273.15;
    const THERMAL_V: f64 = 1.380_649e-23 * TEMP_K / 1.602_176_634e-19;
    let currents = |vbe: f64, vbc: f64| {
        let forward = (vbe / THERMAL_V).exp();
        let reverse = (vbc / THERMAL_V).exp();
        let q2 = IS_A * (forward - 1.0) / ikf_a + IS_A * (reverse - 1.0) / ikr_a;
        let qb = 0.5 * (1.0 + (1.0 + 4.0 * q2).max(0.0).sqrt());
        let transport = IS_A * (forward - reverse) / qb;
        let base_emitter = (IS_A / BF) * (forward - 1.0);
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let collector = transport - base_collector;
        let base = base_emitter + base_collector;
        (collector, base, -(collector + base))
    };

    if !reverse_active {
        const LOAD_OHMS: f64 = 1_000.0;
        const BASE_OHMS: f64 = 2_000.0;
        let collector_for_vbe = |vbe: f64| {
            let (mut low, mut high) = (0.0, SUPPLY_V);
            for _ in 0..100 {
                let collector_v = 0.5 * (low + high);
                let (collector_current, _, _) = currents(vbe, vbe - collector_v);
                let residual = (SUPPLY_V - collector_v) / LOAD_OHMS - collector_current;
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
            let (_, base_current, _) = currents(vbe, vbe - collector_v);
            let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
            if residual > 0.0 {
                low = vbe;
            } else {
                high = vbe;
            }
        }
        return collector_for_vbe(0.5 * (low + high));
    }

    const LOAD_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 2_000.0;
    let emitter_for_vbase = |base_v: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let emitter_v = 0.5 * (low + high);
            let (_, _, emitter_current) = currents(base_v - emitter_v, base_v);
            let residual = (SUPPLY_V - emitter_v) / LOAD_OHMS - emitter_current;
            if residual > 0.0 {
                low = emitter_v;
            } else {
                high = emitter_v;
            }
        }
        0.5 * (low + high)
    };
    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let base_v = 0.5 * (low + high);
        let emitter_v = emitter_for_vbase(base_v);
        let (_, base_current, _) = currents(base_v - emitter_v, base_v);
        let residual = (base_drive_v - base_v) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = base_v;
        } else {
            high = base_v;
        }
    }
    emitter_for_vbase(0.5 * (low + high))
}

/// Independent hot Gummel–Poon load line for the TLEV=3 transport-current
/// temperature law. The only varying model quantity is the effective `IS`.
fn bjt_is_temperature_voltage(base_drive_v: f64, is_a: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000_000.0;
    const BASE_OHMS: f64 = 2_000.0;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const TEMP_K: f64 = 125.0 + 273.15;
    const THERMAL_V: f64 = 1.380_649e-23 * TEMP_K / 1.602_176_634e-19;
    let currents = |vbe: f64, vbc: f64| {
        let forward = (vbe / THERMAL_V).exp();
        let reverse = (vbc / THERMAL_V).exp();
        let transport = is_a * (forward - reverse);
        let base_emitter = (is_a / BF) * (forward - 1.0);
        let base_collector = (is_a / BR) * (reverse - 1.0);
        let collector = transport - base_collector;
        let base = base_emitter + base_collector;
        (collector, base)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = 0.5 * (low + high);
            let (collector_current, _) = currents(vbe, vbe - collector_v);
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
        let (_, base_current) = currents(vbe, vbe - collector_v);
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

/// First backward-Euler step of the two linear CJC partitions. With MJC=0,
/// depletion charge is a constant capacitor; XCJC=0 puts it directly on the
/// external base, while XCJC=1 puts it behind RB at the internal base.
fn bjt_xcjc_first_step_reference() -> (f64, f64) {
    const DRIVE_OHMS: f64 = 1_000.0;
    const RB_OHMS: f64 = 100_000.0;
    const CJC_F: f64 = 10e-9;
    const STEP_S: f64 = 1e-6;
    const INITIAL_V: f64 = 0.2;
    const DRIVE_V: f64 = 1.2;
    let drive_g = 1.0 / DRIVE_OHMS;
    let base_g = 1.0 / RB_OHMS;
    let cap_g = CJC_F / STEP_S;

    let external_partition_v = (drive_g * DRIVE_V + cap_g * INITIAL_V) / (drive_g + cap_g);
    let internal_node_g = drive_g + base_g;
    let internal_base_v = (base_g * drive_g / internal_node_g * DRIVE_V + cap_g * INITIAL_V)
        / (base_g - base_g * base_g / internal_node_g + cap_g);
    let external_base_v = (drive_g * DRIVE_V + base_g * internal_base_v) / internal_node_g;
    (external_base_v, external_partition_v)
}

#[test]
fn nrf5340dk_bjt_high_injection_reaches_saadc_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    assert_eq!(profile.name, "nrf5340dk");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let high_injection = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-high-injection-system.yaml",
    ))
    .expect("load generic matched BJT high-injection branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(high_injection.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    assert!(chip.analog_pins.is_empty());
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 5);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and matched BJT netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAADC channels 0 through 3"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.bjt_high_injection.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_samples = None;
    for base_drive_v in [0.8_f64, 1.5, 3.3] {
        session
            .set_signal_number("ui.bjt_high_injection.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected analog route errors: {errors:?}"
        );

        let ntc_v = voltage(&session, "adc.saadc.0_volts");
        let diode_v = voltage(&session, "adc.saadc.1_volts");
        let rolloff_v = voltage(&session, "adc.saadc.2_volts");
        let control_v = voltage(&session, "adc.saadc.3_volts");
        assert!((ntc_v - expected_ntc_v).abs() < 1e-9);
        assert!((diode_v - expected_diode_v).abs() < 2e-5);
        assert!(
            (rolloff_v - bjt_high_injection_collector_voltage(base_drive_v, true)).abs() < 5e-5,
            "IKF/IKR/NKF load line at {base_drive_v} V must match the independent reference, got {rolloff_v} V"
        );
        assert!(
            (control_v - bjt_high_injection_collector_voltage(base_drive_v, false)).abs() < 5e-5,
            "matched no-rolloff load line at {base_drive_v} V must match the independent reference, got {control_v} V"
        );

        let [ntc_code, diode_code, rolloff_code, control_code, undriven_code] =
            scan_saadc_channels::<5>(&mut bus);
        for (channel, voltage_v, code) in [
            (0, ntc_v, ntc_code),
            (1, diode_v, diode_code),
            (2, rolloff_v, rolloff_code),
            (3, control_v, control_code),
        ] {
            assert_eq!(
                code,
                expected_saadc_code(voltage_v),
                "SAADC channel {channel} converts its routed solved voltage"
            );
        }
        assert_eq!(undriven_code, 0, "neighboring AIN4 stays at ground");
        if base_drive_v == 3.3 {
            high_drive_samples = Some((rolloff_v, control_v));
        }
    }

    let (rolloff_v, control_v) = high_drive_samples.expect("recorded high-drive sample");
    assert!(
        rolloff_v - control_v > 0.05,
        "high-injection rolloff lowers collector current and raises collector voltage: {rolloff_v} V vs {control_v} V"
    );
}

#[test]
fn nrf5340dk_bjt_xcjc_partitions_cjc_across_the_internal_base_resistance() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let xcjc = SystemManifest::from_file(root("examples/nrf5340dk/analog-bjt-xcjc-system.yaml"))
        .expect("load generic BJT XCJC partition branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(xcjc.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 5);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and CJC-partitioned BJT netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic device voltages must resolve to SAADC channels 0 through 3"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");

    let cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve and route NTC, diode, and XCJC BJT branches");
    assert_eq!(steps.len(), 3, "all three generic analog netlists solve");
    assert!(
        errors.is_empty(),
        "unexpected analog route errors: {errors:?}"
    );

    let ntc_v = voltage(&session, "adc.saadc.0_volts");
    let diode_v = voltage(&session, "adc.saadc.1_volts");
    let xcjc_one_external_base_v = voltage(&session, "adc.saadc.2_volts");
    let xcjc_zero_external_base_v = voltage(&session, "adc.saadc.3_volts");
    assert!((ntc_v - 1.65).abs() < 1e-9);
    assert!((diode_v - diode_load_line_voltage()).abs() < 2e-5);

    let (expected_xcjc_one_v, expected_xcjc_zero_v) = bjt_xcjc_first_step_reference();
    assert!(
        (xcjc_one_external_base_v - expected_xcjc_one_v).abs() < 2e-5,
        "XCJC=1 must charge CJC behind RB: expected external base {expected_xcjc_one_v} V, got {xcjc_one_external_base_v} V"
    );
    assert!(
        (xcjc_zero_external_base_v - expected_xcjc_zero_v).abs() < 5e-3,
        "XCJC=0 must charge CJC on the external base: expected {expected_xcjc_zero_v} V, got {xcjc_zero_external_base_v} V"
    );
    assert!(
        xcjc_one_external_base_v - xcjc_zero_external_base_v > 0.45,
        "XCJC changes which side of RB carries the depletion current: {xcjc_one_external_base_v} V vs {xcjc_zero_external_base_v} V"
    );

    let [ntc_code, diode_code, internal_code, external_code, undriven_code] =
        scan_saadc_channels::<5>(&mut bus);
    for (channel, voltage_v, code) in [
        (0, ntc_v, ntc_code),
        (1, diode_v, diode_code),
        (2, xcjc_one_external_base_v, internal_code),
        (3, xcjc_zero_external_base_v, external_code),
    ] {
        assert_eq!(
            code,
            expected_saadc_code(voltage_v),
            "SAADC channel {channel} converts its routed solved voltage"
        );
    }
    assert_eq!(undriven_code, 0, "neighboring AIN4 stays at ground");
}

/// Independent load line of a forward-biased substrate junction: the
/// junction drops `substrate_v − v` and feeds `v / R` into the 100 kΩ load.
/// ngspice's substrate current is `ISS·(e^(v/(NS·Vt)) − 1) + GMIN·v`; the
/// BJT's own junctions are reverse biased and carry ~1e-16 A, below the
/// tolerance.
fn substrate_load_voltage(substrate_v: f64, iss: f64, ns: f64) -> f64 {
    const LOAD_OHMS: f64 = 100_000.0;
    const GMIN: f64 = 1e-12;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
    let (mut low, mut high) = (0.0, substrate_v.max(0.0));
    for _ in 0..200 {
        let load_v = 0.5 * (low + high);
        let junction_v = substrate_v - load_v;
        let substrate_current = iss * ((junction_v / (ns * VT)).exp() - 1.0) + GMIN * junction_v;
        if substrate_current > load_v / LOAD_OHMS {
            low = load_v;
        } else {
            high = load_v;
        }
    }
    0.5 * (low + high)
}

/// Independent backward-Euler load line for a reverse-biased BJT substrate
/// junction. The source resistor and load set the collector's Thevenin load;
/// the charge law is the SPICE depletion integral with FC=0.
fn substrate_charge_step_voltage(
    substrate_v: f64,
    old_charge_c: f64,
    step_s: f64,
    cjs_f: f64,
    vjs_v: f64,
    mjs: f64,
) -> f64 {
    const BIAS_V: f64 = 1.0;
    const SOURCE_OHMS: f64 = 1_000.0;
    const LOAD_OHMS: f64 = 100_000.0;
    const GMIN_S: f64 = 1e-12;

    let charge = |junction_v: f64| {
        -cjs_f * vjs_v / (1.0 - mjs) * ((1.0 - junction_v / vjs_v).powf(1.0 - mjs) - 1.0)
    };
    let residual = |collector_v: f64| {
        let junction_v = substrate_v - collector_v;
        (collector_v - BIAS_V) / SOURCE_OHMS + collector_v / LOAD_OHMS
            - GMIN_S * junction_v
            - (charge(junction_v) - old_charge_c) / step_s
    };
    let (mut low, mut high) = (-1.0, 3.0);
    for _ in 0..120 {
        let collector_v = 0.5 * (low + high);
        if residual(collector_v) > 0.0 {
            high = collector_v;
        } else {
            low = collector_v;
        }
    }
    0.5 * (low + high)
}

fn substrate_depletion_charge(junction_v: f64, cjs_f: f64, vjs_v: f64, mjs: f64) -> f64 {
    -cjs_f * vjs_v / (1.0 - mjs) * ((1.0 - junction_v / vjs_v).powf(1.0 - mjs) - 1.0)
}

/// Independent load line for a hot BJT substrate junction, including the
/// temperature-adjusted diode thermal voltage and the simulator's GMIN path.
fn hot_substrate_load_voltage(substrate_v: f64, iss_a: f64, ns: f64, thermal_v: f64) -> f64 {
    const BIAS_V: f64 = 1.0;
    const SOURCE_OHMS: f64 = 1_000.0;
    const LOAD_OHMS: f64 = 100_000.0;
    const GMIN_S: f64 = 1e-12;

    let residual = |collector_v: f64| {
        let junction_v = substrate_v - collector_v;
        let substrate_current = iss_a * ((junction_v / (ns * thermal_v)).exp() - 1.0);
        (collector_v - BIAS_V) / SOURCE_OHMS + collector_v / LOAD_OHMS
            - substrate_current
            - GMIN_S * junction_v
    };
    let (mut low, mut high) = (-1.0, 3.0);
    for _ in 0..120 {
        let collector_v = 0.5 * (low + high);
        if residual(collector_v) > 0.0 {
            high = collector_v;
        } else {
            low = collector_v;
        }
    }
    0.5 * (low + high)
}

#[test]
fn nrf5340dk_bjt_substrate_junction_reaches_saadc_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let substrate =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-bjt-substrate-system.yaml"))
            .expect("load generic BJT substrate branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(substrate.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 6);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and BJT substrate netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAADC channels 0 through 4"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for substrate_v in [0.0_f64, 0.7, 1.2, 2.0] {
        session
            .set_signal_number("ui.bjt_substrate.substrate_v", substrate_v)
            .expect("set common substrate drive");
        // CJS holds the loaded node for a few 1 µs steps; settle onto the DC
        // load line before sampling.
        for _ in 0..20 {
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .expect("solve and route NTC, diode, and substrate branches");
            assert_eq!(steps.len(), 5, "all five generic analog branches solve");
            assert!(
                errors.is_empty(),
                "unexpected analog route errors: {errors:?}"
            );
        }

        let ntc_v = voltage(&session, "adc.saadc.0_volts");
        let diode_v = voltage(&session, "adc.saadc.1_volts");
        let vertical_v = voltage(&session, "adc.saadc.2_volts");
        let lateral_v = voltage(&session, "adc.saadc.3_volts");
        let control_v = voltage(&session, "adc.saadc.4_volts");
        assert!((ntc_v - 1.65).abs() < 1e-9);
        assert!((diode_v - expected_diode_v).abs() < 2e-5);
        assert!(
            (vertical_v - substrate_load_voltage(substrate_v, 1e-15, 1.2)).abs() < 1e-6,
            "vertical NPN substrate-to-collector load line at {substrate_v} V, got {vertical_v} V"
        );
        assert!(
            (lateral_v - substrate_load_voltage(substrate_v, 2e-15, 1.0)).abs() < 1e-6,
            "lateral PNP substrate-to-base load line at {substrate_v} V, got {lateral_v} V"
        );
        assert!(
            (control_v - substrate_load_voltage(substrate_v, 0.0, 1.0)).abs() < 1e-9,
            "no-ISS control carries only GMIN at {substrate_v} V, got {control_v} V"
        );
        if substrate_v == 2.0 {
            assert!(vertical_v > 1.2 && lateral_v > 1.2 && control_v < 1e-6);
        }

        let [ntc_code, diode_code, vertical_code, lateral_code, control_code, undriven_code] =
            scan_saadc_channels::<6>(&mut bus);
        for (channel, voltage_v, code) in [
            (0, ntc_v, ntc_code),
            (1, diode_v, diode_code),
            (2, vertical_v, vertical_code),
            (3, lateral_v, lateral_code),
            (4, control_v, control_code),
        ] {
            assert_eq!(
                code,
                expected_saadc_code(voltage_v),
                "SAADC channel {channel} converts its routed solved voltage"
            );
        }
        assert_eq!(undriven_code, 0, "neighboring AIN5 stays at ground");
    }
}

#[test]
fn nrf5340dk_bjt_substrate_depletion_charge_reaches_saadc_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let charge = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-substrate-charge-system.yaml",
    ))
    .expect("load matched generic BJT substrate-charge branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 7);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and matched BJT substrate-charge branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAADC channels 0, 1, 5, and 6"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.bjt_substrate_charge.substrate_v", -1.0)
        .expect("initialize reverse-biased substrate junctions");

    let mut cycles = 0;
    for _ in 0..20 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("settle NTC, diode, and matched BJT substrate branches");
        assert_eq!(steps.len(), 4, "all four generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected analog route errors: {errors:?}"
        );
    }

    let old_charge_v = voltage(&session, "adc.saadc.5_volts");
    let old_control_v = voltage(&session, "adc.saadc.6_volts");
    assert!(
        (old_charge_v - old_control_v).abs() < 1e-6,
        "CJS changes no DC current when ISS=0: {old_charge_v} V vs {old_control_v} V"
    );
    const CJS_F: f64 = 100e-9;
    const VJS_V: f64 = 0.6;
    const MJS: f64 = 0.4;
    const STEP_S: f64 = 1e-6;
    const OLD_SUBSTRATE_V: f64 = -1.0;
    const NEW_SUBSTRATE_V: f64 = -0.8;
    let old_junction_v = OLD_SUBSTRATE_V - old_charge_v;
    let old_charge_c = substrate_depletion_charge(old_junction_v, CJS_F, VJS_V, MJS);

    session
        .set_signal_number("ui.bjt_substrate_charge.substrate_v", NEW_SUBSTRATE_V)
        .expect("step substrate voltage while the junction is reverse biased");
    cycles += session.cycles_until_boundary(cycles);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve BJT substrate displacement-current transient");
    assert_eq!(steps.len(), 4, "all four generic analog branches solve");
    assert!(
        errors.is_empty(),
        "unexpected analog route errors: {errors:?}"
    );

    let ntc_v = voltage(&session, "adc.saadc.0_volts");
    let diode_v = voltage(&session, "adc.saadc.1_volts");
    let charge_v = voltage(&session, "adc.saadc.5_volts");
    let control_v = voltage(&session, "adc.saadc.6_volts");
    let expected_charge_v =
        substrate_charge_step_voltage(NEW_SUBSTRATE_V, old_charge_c, STEP_S, CJS_F, VJS_V, MJS);
    assert!(
        (charge_v - expected_charge_v).abs() < 50e-6,
        "BJT CJS transient follows independent backward-Euler depletion-charge load line: expected {expected_charge_v} V, got {charge_v} V"
    );
    assert!(
        (control_v - old_control_v).abs() < 1e-6,
        "zero-CJS control stays at its DC load line: {old_control_v} V -> {control_v} V"
    );
    assert!(
        charge_v > control_v + 0.1,
        "substrate depletion charge moves the collector while the zero-charge control does not: {charge_v} V vs {control_v} V"
    );
    assert!(
        (ntc_v - 1.65).abs() < 1e-9,
        "unrelated NTC remains isolated"
    );
    assert!(
        (diode_v - diode_load_line_voltage()).abs() < 2e-5,
        "unrelated Shockley diode remains isolated"
    );

    let codes = scan_saadc_channels::<7>(&mut bus);
    for (channel, voltage_v) in [(0, ntc_v), (1, diode_v), (5, charge_v), (6, control_v)] {
        assert_eq!(
            codes[channel],
            expected_saadc_code(voltage_v),
            "SAADC channel {channel} converts its routed solved voltage"
        );
    }
    assert_eq!(codes[2], 0, "unused AIN2 stays at ground");
    assert_eq!(codes[3], 0, "unused AIN3 stays at ground");
    assert_eq!(codes[4], 0, "unused AIN4 stays at ground");
}

#[test]
fn nrf5340dk_bjt_substrate_temperature_coefficients_reach_saadc() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let temperature = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-substrate-temperature-system.yaml",
    ))
    .expect("load generic BJT substrate temperature-coefficient branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(temperature.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 8);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and BJT temperature-coefficient branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAADC channels 0, 1, and 5 through 7"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.bjt_substrate_temperature.substrate_v", -1.0)
        .expect("initialize reverse-biased substrate junctions");

    let mut cycles = 0;
    for _ in 0..20 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("settle NTC, diode, and BJT substrate branches");
        assert_eq!(steps.len(), 5, "all five generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected analog route errors: {errors:?}"
        );
    }

    const NOMINAL_CJS_F: f64 = 10e-9;
    const NOMINAL_VJS_V: f64 = 0.6;
    const NOMINAL_MJS: f64 = 0.4;
    const TNOM_C: f64 = 27.0;
    const TEMP_C: f64 = 125.0;
    const CTS_PER_C: f64 = 5e-3;
    const TVJS_V_PER_C: f64 = 1e-3;
    const TMJS_PER_C: f64 = 2e-3;
    const NEW_SUBSTRATE_V: f64 = -0.8;
    let delta_c = TEMP_C - TNOM_C;
    let hot_cjs_f = NOMINAL_CJS_F * (1.0 + CTS_PER_C * delta_c);
    let hot_vjs_v = NOMINAL_VJS_V - TVJS_V_PER_C * delta_c;
    let hot_mjs = NOMINAL_MJS * (1.0 + TMJS_PER_C * delta_c);
    assert!((hot_cjs_f - 14.9e-9).abs() < 1e-20);
    assert!((hot_vjs_v - 0.502).abs() < 1e-12);
    assert!((hot_mjs - 0.4784).abs() < 1e-12);

    let old_hot_v = voltage(&session, "adc.saadc.5_volts");
    let old_explicit_v = voltage(&session, "adc.saadc.6_volts");
    let old_nominal_v = voltage(&session, "adc.saadc.7_volts");
    for old_v in [old_hot_v, old_explicit_v, old_nominal_v] {
        assert!(
            (old_v - old_nominal_v).abs() < 1e-6,
            "CJS/VJS/MJS alter no DC current with ISS=0: {old_v} V vs {old_nominal_v} V"
        );
    }

    session
        .set_signal_number("ui.bjt_substrate_temperature.substrate_v", NEW_SUBSTRATE_V)
        .expect("step substrate voltage while each junction is reverse biased");
    cycles += session.cycles_until_boundary(cycles);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve temperature-adjusted substrate displacement currents");
    assert_eq!(steps.len(), 5, "all five generic analog branches solve");
    assert!(
        errors.is_empty(),
        "unexpected analog route errors: {errors:?}"
    );

    let ntc_v = voltage(&session, "adc.saadc.0_volts");
    let diode_v = voltage(&session, "adc.saadc.1_volts");
    let hot_v = voltage(&session, "adc.saadc.5_volts");
    let explicit_v = voltage(&session, "adc.saadc.6_volts");
    let nominal_v = voltage(&session, "adc.saadc.7_volts");
    assert!(
        (hot_v - explicit_v).abs() < 50e-6,
        "TLEVC=1 CTS/TVJS/TMJS1 branch must match the explicit hot CJS/VJS/MJS card: {hot_v} V vs {explicit_v} V"
    );
    assert!(
        (hot_v - nominal_v).abs() > 5e-3,
        "temperature-adjusted CJS/VJS/MJS must move the solved transient, got hot {hot_v} V and nominal {nominal_v} V"
    );
    assert!(
        (ntc_v - 1.65).abs() < 1e-9,
        "unrelated NTC remains isolated"
    );
    assert!(
        (diode_v - diode_load_line_voltage()).abs() < 2e-5,
        "unrelated Shockley diode remains isolated"
    );

    let codes = scan_saadc_channels::<8>(&mut bus);
    for (channel, voltage_v) in [
        (0, ntc_v),
        (1, diode_v),
        (5, hot_v),
        (6, explicit_v),
        (7, nominal_v),
    ] {
        assert_eq!(
            codes[channel],
            expected_saadc_code(voltage_v),
            "SAADC channel {channel} converts its routed solved voltage"
        );
    }
    for channel in [2, 3, 4] {
        assert_eq!(codes[channel], 0, "unused AIN{channel} stays at ground");
    }
}

#[test]
fn nrf5340dk_bjt_substrate_tiss_temperature_coefficients_reach_saadc() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let tiss = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-substrate-tiss-system.yaml",
    ))
    .expect("load generic BJT substrate-current temperature branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(tiss.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 8);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and BJT substrate TISS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAADC channels 0 through 5"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.bjt_substrate_tiss.substrate_v", 2.0)
        .expect("forward-bias each substrate junction");

    let mut cycles = 0;
    for _ in 0..20 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("settle NTC, diode, and BJT substrate-current branches");
        assert_eq!(steps.len(), 6, "all six generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected analog route errors: {errors:?}"
        );
    }

    const ISS_A: f64 = 1e-15;
    const NS: f64 = 1.2;
    const TNOM_C: f64 = 27.0;
    const TEMP_C: f64 = 125.0;
    const TISS1_PER_C: f64 = -2e-3;
    const TISS2_PER_C2: f64 = 1e-5;
    const SUBSTRATE_V: f64 = 2.0;
    const BOLTZMANN_OVER_Q_V_PER_K: f64 = 1.380_649e-23 / 1.602_176_634e-19;
    let delta_c = TEMP_C - TNOM_C;
    let hot_exponent = 1.0 + TISS1_PER_C * delta_c + TISS2_PER_C2 * delta_c * delta_c;
    let hot_iss_a = ISS_A.powf(hot_exponent);
    let first_order_iss_a = ISS_A.powf(1.0 + TISS1_PER_C * delta_c);
    let nominal_iss_a = ISS_A;
    let thermal_v = BOLTZMANN_OVER_Q_V_PER_K * (TEMP_C + 273.15);
    assert!((hot_exponent - 0.90004).abs() < 1e-12);
    assert!((hot_iss_a - 3.157_911_828_632_245e-14).abs() < 1e-25);

    let ntc_v = voltage(&session, "adc.saadc.0_volts");
    let diode_v = voltage(&session, "adc.saadc.1_volts");
    let coefficient_v = voltage(&session, "adc.saadc.2_volts");
    let explicit_v = voltage(&session, "adc.saadc.3_volts");
    let first_order_v = voltage(&session, "adc.saadc.4_volts");
    let nominal_v = voltage(&session, "adc.saadc.5_volts");
    for (name, actual_v, iss_a) in [
        ("TISS1/TISS2", coefficient_v, hot_iss_a),
        ("explicit hot ISS", explicit_v, hot_iss_a),
        ("TISS1 only", first_order_v, first_order_iss_a),
        ("nominal ISS", nominal_v, nominal_iss_a),
    ] {
        let expected_v = hot_substrate_load_voltage(SUBSTRATE_V, iss_a, NS, thermal_v);
        assert!(
            (actual_v - expected_v).abs() < 1e-6,
            "{name} substrate load line should be {expected_v} V, got {actual_v} V"
        );
    }
    assert!(
        (coefficient_v - explicit_v).abs() < 1e-6,
        "TLEV=3 TISS1/TISS2 must match the explicitly temperature-adjusted ISS: {coefficient_v} V vs {explicit_v} V"
    );
    assert!(
        (coefficient_v - first_order_v).abs() > 0.05,
        "TISS2 must change the hot substrate load line beyond the TISS1-only card: {coefficient_v} V vs {first_order_v} V"
    );
    assert!(
        (coefficient_v - nominal_v).abs() > 0.05,
        "hot TISS coefficients must change the substrate load line from nominal ISS: {coefficient_v} V vs {nominal_v} V"
    );
    assert!(
        (ntc_v - 1.65).abs() < 1e-9,
        "unrelated NTC remains isolated"
    );
    assert!(
        (diode_v - diode_load_line_voltage()).abs() < 2e-5,
        "unrelated Shockley diode remains isolated"
    );

    let codes = scan_saadc_channels::<8>(&mut bus);
    for (channel, voltage_v) in [
        (0, ntc_v),
        (1, diode_v),
        (2, coefficient_v),
        (3, explicit_v),
        (4, first_order_v),
        (5, nominal_v),
    ] {
        assert_eq!(
            codes[channel],
            expected_saadc_code(voltage_v),
            "SAADC channel {channel} converts its routed solved voltage"
        );
    }
    for channel in [6, 7] {
        assert_eq!(codes[channel], 0, "unused AIN{channel} stays at ground");
    }
}

#[test]
fn nrf5340dk_ntc_and_diode_netlists_reach_distinct_saadc_channels() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    assert_eq!(profile.name, "nrf5340dk");
    assert!(profile.external_devices.is_empty());
    assert_eq!(
        profile
            .board_io
            .iter()
            .map(|io| (io.id.as_str(), io.pin))
            .collect::<Vec<_>>(),
        [
            ("led1", 28),
            ("led2", 29),
            ("led3", 30),
            ("led4", 31),
            ("button1", 23),
            ("button2", 24),
            ("button3", 8),
            ("button4", 9),
        ],
        "the analog test must retain the DK's normal LED and button wiring"
    );

    let analog_path = root("examples/nrf5340dk/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    assert!(
        chip.analog_pins.is_empty(),
        "use explicit SAADC AIN routes rather than claiming unverified pad aliases"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries two analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic analog device outputs must bind to SAADC AIN0 and AIN1"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both device netlists");
        assert_eq!(steps.len(), 2, "both independent analog devices solve");
        assert!(
            errors.is_empty(),
            "unexpected analog route errors: {errors:?}"
        );

        let ntc_v = voltage(&session, "adc.saadc.0_volts");
        let ntc_r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * ntc_r / (ntc_r + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let [ntc_code, diode_code, undriven_code] = scan(&mut bus);
        assert_eq!(
            ntc_code,
            expected_saadc_code(ntc_v),
            "SAADC AIN0 must convert the solved temperature-dependent voltage"
        );
        assert_eq!(
            diode_code,
            expected_saadc_code(diode_v),
            "SAADC AIN1 must independently convert the Shockley load line"
        );
        assert_eq!(
            undriven_code, 0,
            "the routed but undriven SAADC AIN2 channel stays at ground"
        );
        samples.push((temperature_c, ntc_v, ntc_code, diode_code));
    }

    assert!(
        samples[0].1 > samples[1].1 && samples[1].1 > samples[2].1,
        "the NTC divider voltage should fall as the NTC heats: {samples:?}"
    );
    assert!(
        samples[0].2 > samples[1].2 && samples[1].2 > samples[2].2,
        "the SAADC result should track the temperature-dependent voltage: {samples:?}"
    );
    assert!(
        samples.iter().all(|sample| sample.3 == samples[0].3),
        "the independent diode channel should stay constant while the NTC changes: {samples:?}"
    );
}

#[test]
fn nrf5340dk_bjt_base_resistance_temperature_coefficients_reach_saadc() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let base_resistance = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-base-resistance-temperature-system.yaml",
    ))
    .expect("load matched BJT RBM/IRB temperature-coefficient branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(base_resistance.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 8);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and five BJT resistance branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve to SAADC channels 0 through 6"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_base_resistance_temperature.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    const RBM_NOMINAL_OHMS: f64 = 1_000.0;
    const IRB_NOMINAL_A: f64 = 20e-6;
    // TNOM=25 °C, TEMP=125 °C: TRBM1/2 produce ×1.2 and TIRB1/2
    // produce ×1.3. The explicit branch is a second encoding of those values.
    const RBM_HOT_OHMS: f64 = 1_200.0;
    const IRB_HOT_A: f64 = 26e-6;
    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_voltages = None;

    for base_drive_v in [0.9_f64, 1.0, 1.1, 1.2] {
        session
            .set_signal_number("ui.bjt_base_resistance_temperature.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and temperature-adjusted BJT branches");
        assert_eq!(steps.len(), 7, "all seven generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected SAADC route errors: {errors:?}"
        );

        let routed_ntc_v = voltage(&session, "adc.saadc.0_volts");
        let routed_diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!((routed_ntc_v - ntc_v).abs() < 1e-9);
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the independent Shockley diode remains isolated"
        );

        let actual = [
            voltage(&session, "adc.saadc.2_volts"),
            voltage(&session, "adc.saadc.3_volts"),
            voltage(&session, "adc.saadc.4_volts"),
            voltage(&session, "adc.saadc.5_volts"),
            voltage(&session, "adc.saadc.6_volts"),
        ];
        let expected = [
            bjt_base_resistance_temperature_collector_voltage(
                base_drive_v,
                RBM_HOT_OHMS,
                IRB_HOT_A,
                125.0,
            ),
            bjt_base_resistance_temperature_collector_voltage(
                base_drive_v,
                RBM_HOT_OHMS,
                IRB_HOT_A,
                125.0,
            ),
            bjt_base_resistance_temperature_collector_voltage(
                base_drive_v,
                RBM_HOT_OHMS,
                IRB_NOMINAL_A,
                125.0,
            ),
            bjt_base_resistance_temperature_collector_voltage(
                base_drive_v,
                RBM_NOMINAL_OHMS,
                IRB_HOT_A,
                125.0,
            ),
            bjt_base_resistance_temperature_collector_voltage(
                base_drive_v,
                RBM_NOMINAL_OHMS,
                IRB_NOMINAL_A,
                125.0,
            ),
        ];
        for (channel, (actual_v, expected_v)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                (actual_v - expected_v).abs() < 5e-5,
                "SAADC channel {} independent RBM/IRB load line at {base_drive_v} V should be {expected_v} V, got {actual_v} V",
                channel + 2
            );
        }
        assert!(
            (actual[0] - actual[1]).abs() < 1e-9,
            "TRBM/TIRB coefficients must match an explicit hot RBM/IRB model card"
        );

        let codes = scan_saadc_channels::<8>(&mut bus);
        for (channel, voltage_v) in [
            (0, routed_ntc_v),
            (1, routed_diode_v),
            (2, actual[0]),
            (3, actual[1]),
            (4, actual[2]),
            (5, actual[3]),
            (6, actual[4]),
        ] {
            assert_eq!(
                codes[channel],
                expected_saadc_code(voltage_v),
                "SAADC channel {channel} converts its routed solved voltage"
            );
        }
        assert_eq!(codes[7], 0, "neighboring undriven AIN7 stays at ground");

        if base_drive_v == 1.2 {
            high_drive_voltages = Some(actual);
        }
    }

    let [both, explicit, trbm_only, tirb_only, nominal] =
        high_drive_voltages.expect("high-drive sample was recorded");
    assert!(
        trbm_only - nominal > 0.01,
        "TRBM temperature coefficients raise the high-drive collector voltage: {trbm_only} V vs {nominal} V"
    );
    assert!(
        tirb_only - nominal > 0.05,
        "TIRB temperature coefficients independently raise the high-drive collector voltage: {tirb_only} V vs {nominal} V"
    );
    assert!(
        both - nominal > 0.05,
        "combined TRBM/TIRB temperature laws shift the high-drive load line: {both} V vs {nominal} V"
    );
    assert!((both - explicit).abs() < 1e-9);
}

#[test]
fn nrf5340dk_bjt_recombination_temperature_coefficients_reach_saadc() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let recombination = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-recombination-temperature-system.yaml",
    ))
    .expect("load matched BJT recombination temperature branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(recombination.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 8);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and six generic BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAADC AIN0 through AIN7"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_recombination_temperature.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    const TEMP_C: f64 = 125.0;
    const TNOM_C: f64 = 25.0;
    const ISE_A: f64 = 2e-12;
    const ISC_A: f64 = 50e-9;
    const TISE1: f64 = -2e-3;
    const TISE2: f64 = 10e-6;
    const TISC1: f64 = -1e-3;
    const TISC2: f64 = 8e-6;
    let delta_c = TEMP_C - TNOM_C;
    let hot_ise_a = ISE_A.powf(1.0 + TISE1 * delta_c + TISE2 * delta_c * delta_c);
    let hot_isc_a = ISC_A.powf(1.0 + TISC1 * delta_c + TISC2 * delta_c * delta_c);
    let first_order_ise_a = ISE_A.powf(1.0 + TISE1 * delta_c);
    let first_order_isc_a = ISC_A.powf(1.0 + TISC1 * delta_c);
    assert!((hot_ise_a - 2.957_515_273_256_625_5e-11).abs() < 1e-24);
    assert!((hot_isc_a - 6.998_268_554_235_417e-8).abs() < 1e-21);
    assert!((first_order_ise_a - 4.373_448_295_773_107e-10).abs() < 1e-23);
    assert!((first_order_isc_a - 2.685_795_883_818_437_5e-7).abs() < 1e-20);

    let mut cycles = 0;
    cycles += session.cycles_until_boundary(cycles);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize the NTC, diode, and BJT branches");
    assert_eq!(
        initial_steps.len(),
        8,
        "all eight generic analog branches solve"
    );
    assert!(
        initial_errors.is_empty(),
        "unexpected SAADC route errors: {initial_errors:?}"
    );

    let mut high_drive_voltages = None;
    for base_drive_v in [1.2_f64, 1.4, 1.6, 1.8] {
        session
            .set_signal_number("ui.bjt_recombination_temperature.base_v", base_drive_v)
            .expect("set common base drive for matched BJT branches");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and hot BJT recombination branches");
        assert_eq!(steps.len(), 8, "all eight generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected SAADC route errors: {errors:?}"
        );

        let ntc_v = voltage(&session, "adc.saadc.0_volts");
        let diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "unrelated NTC remains isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "unrelated Shockley diode remains on its independent load line"
        );

        let actual = [
            voltage(&session, "adc.saadc.2_volts"),
            voltage(&session, "adc.saadc.3_volts"),
            voltage(&session, "adc.saadc.4_volts"),
            voltage(&session, "adc.saadc.5_volts"),
            voltage(&session, "adc.saadc.6_volts"),
            voltage(&session, "adc.saadc.7_volts"),
        ];
        let expected = [
            (hot_ise_a, hot_isc_a),
            (hot_ise_a, hot_isc_a),
            (first_order_ise_a, first_order_isc_a),
            (hot_ise_a, ISC_A),
            (ISE_A, hot_isc_a),
            (ISE_A, ISC_A),
        ]
        .map(|(ise_a, isc_a)| {
            bjt_recombination_temperature_collector_voltage(base_drive_v, ise_a, isc_a, TEMP_C)
        });
        for (index, (actual_v, expected_v)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                (actual_v - expected_v).abs() < 5e-5,
                "SAADC AIN{} independent hot recombination load line at {base_drive_v} V should be {expected_v} V, got {actual_v} V",
                index + 2
            );
        }
        assert!(
            (actual[0] - actual[1]).abs() < 1e-9,
            "TISE/TISC coefficients must match explicitly raised hot ISE/ISC values"
        );

        let codes = scan_saadc_channels::<8>(&mut bus);
        for (channel, voltage_v) in [
            (0, ntc_v),
            (1, diode_v),
            (2, actual[0]),
            (3, actual[1]),
            (4, actual[2]),
            (5, actual[3]),
            (6, actual[4]),
            (7, actual[5]),
        ] {
            assert_eq!(
                codes[channel],
                expected_saadc_code(voltage_v),
                "SAADC AIN{channel} converts its routed solved voltage"
            );
        }

        if base_drive_v == 1.8 {
            high_drive_voltages = Some(actual);
        }
    }

    let [combined, explicit, first_order, be_temperature, bc_temperature, nominal] =
        high_drive_voltages.expect("high-drive sample was recorded");
    assert!((combined - explicit).abs() < 1e-9);
    assert!(
        (combined - first_order).abs() > 0.01,
        "TISE2/TISC2 must change the first-order-only hot load line"
    );
    assert!(
        (be_temperature - nominal).abs() > 0.01,
        "TISE1/TISE2 independently change base-emitter recombination"
    );
    assert!(
        (bc_temperature - nominal).abs() > 0.01,
        "TISC1/TISC2 independently change base-collector recombination"
    );
    assert!(
        (combined - be_temperature).abs() > 0.01,
        "base-collector coefficient changes remain visible beside base-emitter coefficients"
    );
    assert!(
        (combined - bc_temperature).abs() > 0.01,
        "base-emitter coefficient changes remain visible beside base-collector coefficients"
    );
}

#[test]
fn nrf5340dk_bjt_beta_temperature_coefficients_reach_saadc() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let beta = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-beta-temperature-system.yaml",
    ))
    .expect("load matched forward/reverse beta temperature branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(beta.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 8);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and six generic BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve to SAADC AIN0 through AIN7"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_beta_temperature.forward_base_v", 0.0)
        .expect("initialize forward-active BJT base drives");
    session
        .set_signal_number("ui.bjt_beta_temperature.reverse_base_v", 0.0)
        .expect("initialize reverse-active BJT base drives");

    const TNOM_C: f64 = 25.0;
    const TEMP_C: f64 = 125.0;
    const DELTA_C: f64 = TEMP_C - TNOM_C;
    const BF_NOMINAL: f64 = 80.0;
    const BR_NOMINAL: f64 = 4.0;
    const TBF1: f64 = 2e-3;
    const TBF2: f64 = 15e-6;
    const TBR1: f64 = 3e-3;
    const TBR2: f64 = 30e-6;
    let bf_hot = BF_NOMINAL * (1.0 + TBF1 * DELTA_C + TBF2 * DELTA_C * DELTA_C);
    let bf_first_order = BF_NOMINAL * (1.0 + TBF1 * DELTA_C);
    let br_hot = BR_NOMINAL * (1.0 + TBR1 * DELTA_C + TBR2 * DELTA_C * DELTA_C);
    let br_first_order = BR_NOMINAL * (1.0 + TBR1 * DELTA_C);
    assert!((bf_hot - 108.0).abs() < 1e-12);
    assert!((bf_first_order - 96.0).abs() < 1e-12);
    assert!((br_hot - 6.4).abs() < 1e-12);
    assert!((br_first_order - 5.2).abs() < 1e-12);

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_voltages = None;
    for (forward_drive_v, reverse_drive_v) in [(1.0_f64, 0.9_f64), (1.1, 1.0), (1.2, 1.1)] {
        session
            .set_signal_number("ui.bjt_beta_temperature.forward_base_v", forward_drive_v)
            .expect("set common forward-active base drive");
        session
            .set_signal_number("ui.bjt_beta_temperature.reverse_base_v", reverse_drive_v)
            .expect("set common reverse-active base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and hot BJT beta branches");
        assert_eq!(steps.len(), 8, "all eight generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected SAADC route errors: {errors:?}"
        );

        let routed_ntc_v = voltage(&session, "adc.saadc.0_volts");
        let routed_diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!((routed_ntc_v - ntc_v).abs() < 1e-9);
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the independent Shockley diode remains isolated"
        );
        let actual = [
            voltage(&session, "adc.saadc.2_volts"),
            voltage(&session, "adc.saadc.3_volts"),
            voltage(&session, "adc.saadc.4_volts"),
            voltage(&session, "adc.saadc.5_volts"),
            voltage(&session, "adc.saadc.6_volts"),
            voltage(&session, "adc.saadc.7_volts"),
        ];
        let expected = [
            bjt_beta_temperature_voltage(forward_drive_v, bf_hot, BR_NOMINAL, false),
            bjt_beta_temperature_voltage(forward_drive_v, bf_hot, BR_NOMINAL, false),
            bjt_beta_temperature_voltage(forward_drive_v, bf_first_order, BR_NOMINAL, false),
            bjt_beta_temperature_voltage(reverse_drive_v, BF_NOMINAL, br_hot, true),
            bjt_beta_temperature_voltage(reverse_drive_v, BF_NOMINAL, br_hot, true),
            bjt_beta_temperature_voltage(reverse_drive_v, BF_NOMINAL, br_first_order, true),
        ];
        for (index, (actual_v, expected_v)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                (actual_v - expected_v).abs() < 5e-5,
                "SAADC AIN{} independent beta temperature load line should be {expected_v} V, got {actual_v} V",
                index + 2
            );
        }
        assert!(
            (actual[0] - actual[1]).abs() < 1e-9,
            "TBF1/TBF2 must match an explicit hot BF model card"
        );
        assert!(
            (actual[3] - actual[4]).abs() < 1e-9,
            "TBR1/TBR2 must match an explicit hot BR model card"
        );

        let codes = scan_saadc_channels::<8>(&mut bus);
        for (channel, voltage_v) in [
            (0, routed_ntc_v),
            (1, routed_diode_v),
            (2, actual[0]),
            (3, actual[1]),
            (4, actual[2]),
            (5, actual[3]),
            (6, actual[4]),
            (7, actual[5]),
        ] {
            assert_eq!(
                codes[channel],
                expected_saadc_code(voltage_v),
                "SAADC AIN{channel} converts its routed solved voltage"
            );
        }
        if forward_drive_v == 1.2 {
            high_drive_voltages = Some(actual);
        }
    }

    let [bf_full, bf_explicit, bf_first, br_full, br_explicit, br_first] =
        high_drive_voltages.expect("high-drive sample was recorded");
    assert!((bf_full - bf_explicit).abs() < 1e-9);
    assert!(
        (bf_full - bf_first).abs() > 0.01,
        "TBF2 must change the forward high-drive load line: {bf_full} V vs {bf_first} V"
    );
    assert!((br_full - br_explicit).abs() < 1e-9);
    assert!(
        (br_full - br_first).abs() > 0.01,
        "TBR2 must change the reverse-active high-drive load line: {br_full} V vs {br_first} V"
    );
}

#[test]
fn nrf5340dk_bjt_injection_knee_temperature_coefficients_reach_saadc() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let knees = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-knee-temperature-system.yaml",
    ))
    .expect("load matched forward/reverse high-injection temperature branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(knees.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 8);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and six generic BJT knee branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve to SAADC AIN0 through AIN7"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_knee_temperature.forward_base_v", 0.0)
        .expect("initialize forward-active BJT base drives");
    session
        .set_signal_number("ui.bjt_knee_temperature.reverse_base_v", 0.0)
        .expect("initialize reverse-active BJT base drives");

    const IKF_NOMINAL_A: f64 = 50e-6;
    const IKR_NOMINAL_A: f64 = 500e-6;
    const IKF_HOT_A: f64 = IKF_NOMINAL_A * (1.0 - 1e-3 * 100.0 + 20e-6 * 100.0 * 100.0);
    const IKF_FIRST_ORDER_A: f64 = IKF_NOMINAL_A * (1.0 - 1e-3 * 100.0);
    const IKR_HOT_A: f64 = IKR_NOMINAL_A * (1.0 + 1e-3 * 100.0 + 50e-6 * 100.0 * 100.0);
    const IKR_FIRST_ORDER_A: f64 = IKR_NOMINAL_A * (1.0 + 1e-3 * 100.0);
    assert!((IKF_HOT_A - 55e-6).abs() < 1e-18);
    assert!((IKF_FIRST_ORDER_A - 45e-6).abs() < 1e-18);
    assert!((IKR_HOT_A - 800e-6).abs() < 1e-18);
    assert!((IKR_FIRST_ORDER_A - 550e-6).abs() < 1e-18);

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_voltages = None;
    for (forward_drive_v, reverse_drive_v) in [(1.0_f64, 0.9_f64), (1.2, 1.0), (1.4, 1.1)] {
        session
            .set_signal_number("ui.bjt_knee_temperature.forward_base_v", forward_drive_v)
            .expect("set common forward-active base drive");
        session
            .set_signal_number("ui.bjt_knee_temperature.reverse_base_v", reverse_drive_v)
            .expect("set common reverse-active base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and hot BJT knee branches");
        assert_eq!(steps.len(), 8, "all eight generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected SAADC route errors: {errors:?}"
        );

        let routed_ntc_v = voltage(&session, "adc.saadc.0_volts");
        let routed_diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!((routed_ntc_v - ntc_v).abs() < 1e-9);
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the independent Shockley diode remains isolated"
        );
        let actual = [
            voltage(&session, "adc.saadc.2_volts"),
            voltage(&session, "adc.saadc.3_volts"),
            voltage(&session, "adc.saadc.4_volts"),
            voltage(&session, "adc.saadc.5_volts"),
            voltage(&session, "adc.saadc.6_volts"),
            voltage(&session, "adc.saadc.7_volts"),
        ];
        let expected = [
            bjt_knee_temperature_voltage(forward_drive_v, IKF_HOT_A, IKR_NOMINAL_A, false),
            bjt_knee_temperature_voltage(forward_drive_v, IKF_HOT_A, IKR_NOMINAL_A, false),
            bjt_knee_temperature_voltage(forward_drive_v, IKF_FIRST_ORDER_A, IKR_NOMINAL_A, false),
            bjt_knee_temperature_voltage(reverse_drive_v, IKF_NOMINAL_A, IKR_HOT_A, true),
            bjt_knee_temperature_voltage(reverse_drive_v, IKF_NOMINAL_A, IKR_HOT_A, true),
            bjt_knee_temperature_voltage(reverse_drive_v, IKF_NOMINAL_A, IKR_FIRST_ORDER_A, true),
        ];
        for (index, (actual_v, expected_v)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                (actual_v - expected_v).abs() < 5e-5,
                "SAADC AIN{} independent temperature-adjusted knee load line should be {expected_v} V, got {actual_v} V",
                index + 2
            );
        }
        assert!(
            (actual[0] - actual[1]).abs() < 1e-9,
            "TIKF1/TIKF2 must match the explicit hot IKF model card"
        );
        assert!(
            (actual[3] - actual[4]).abs() < 1e-9,
            "TIKR1/TIKR2 must match the explicit hot IKR model card"
        );

        let codes = scan_saadc_channels::<8>(&mut bus);
        for (channel, voltage_v) in [
            (0, routed_ntc_v),
            (1, routed_diode_v),
            (2, actual[0]),
            (3, actual[1]),
            (4, actual[2]),
            (5, actual[3]),
            (6, actual[4]),
            (7, actual[5]),
        ] {
            assert_eq!(
                codes[channel],
                expected_saadc_code(voltage_v),
                "SAADC AIN{channel} converts its routed solved voltage"
            );
        }
        if forward_drive_v == 1.4 {
            high_drive_voltages = Some(actual);
        }
    }

    let [ikf_full, ikf_explicit, ikf_first, ikr_full, ikr_explicit, ikr_first] =
        high_drive_voltages.expect("high-drive sample was recorded");
    assert!((ikf_full - ikf_explicit).abs() < 1e-9);
    assert!(
        (ikf_full - ikf_first).abs() > 0.01,
        "TIKF2 must change the forward high-injection load line: {ikf_full} V vs {ikf_first} V"
    );
    assert!((ikr_full - ikr_explicit).abs() < 1e-9);
    assert!(
        (ikr_full - ikr_first).abs() > 0.01,
        "TIKR2 must change the reverse-active high-injection load line: {ikr_full} V vs {ikr_first} V"
    );
}

#[test]
fn nrf5340dk_bjt_transport_is_temperature_coefficients_reach_saadc() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let transport_is = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-is-temperature-system.yaml",
    ))
    .expect("load matched TIS1/TIS2 transport-current branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(transport_is.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 6);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and four generic BJT TIS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve to SAADC AIN0 through AIN5"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_is_temperature.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    const IS_NOMINAL_A: f64 = 1e-14;
    const TIS1_PER_C: f64 = -1e-3;
    const TIS2_PER_C2: f64 = 6e-6;
    const DELTA_C: f64 = 100.0;
    let hot_exponent = 1.0 + TIS1_PER_C * DELTA_C + TIS2_PER_C2 * DELTA_C * DELTA_C;
    let first_order_exponent = 1.0 + TIS1_PER_C * DELTA_C;
    let is_hot_a = IS_NOMINAL_A.powf(hot_exponent);
    let is_first_order_a = IS_NOMINAL_A.powf(first_order_exponent);
    assert!((hot_exponent - 0.96).abs() < 1e-15);
    assert!((first_order_exponent - 0.9).abs() < 1e-15);

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_voltages = None;
    for base_drive_v in [0.35_f64, 0.4, 0.45] {
        session
            .set_signal_number("ui.bjt_is_temperature.base_v", base_drive_v)
            .expect("set common forward-active base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and hot BJT transport-current branches");
        assert_eq!(steps.len(), 6, "all six generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected SAADC route errors: {errors:?}"
        );

        let routed_ntc_v = voltage(&session, "adc.saadc.0_volts");
        let routed_diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!((routed_ntc_v - ntc_v).abs() < 1e-9);
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the independent Shockley diode remains isolated"
        );
        let actual = [
            voltage(&session, "adc.saadc.2_volts"),
            voltage(&session, "adc.saadc.3_volts"),
            voltage(&session, "adc.saadc.4_volts"),
            voltage(&session, "adc.saadc.5_volts"),
        ];
        let expected = [
            bjt_is_temperature_voltage(base_drive_v, is_hot_a),
            bjt_is_temperature_voltage(base_drive_v, is_hot_a),
            bjt_is_temperature_voltage(base_drive_v, is_first_order_a),
            bjt_is_temperature_voltage(base_drive_v, IS_NOMINAL_A),
        ];
        for (index, (actual_v, expected_v)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                (actual_v - expected_v).abs() < 5e-5,
                "SAADC AIN{} independent TIS-adjusted load line should be {expected_v} V, got {actual_v} V",
                index + 2
            );
        }
        assert!(
            (actual[0] - actual[1]).abs() < 1e-9,
            "TIS1/TIS2 must match the explicit hot IS model card: {actual:?}"
        );

        let codes = scan_saadc_channels::<6>(&mut bus);
        for (channel, voltage_v) in [
            (0, routed_ntc_v),
            (1, routed_diode_v),
            (2, actual[0]),
            (3, actual[1]),
            (4, actual[2]),
            (5, actual[3]),
        ] {
            assert_eq!(
                codes[channel],
                expected_saadc_code(voltage_v),
                "SAADC AIN{channel} converts its routed solved voltage"
            );
        }
        if base_drive_v == 0.45 {
            high_drive_voltages = Some(actual);
        }
    }

    let [full, explicit, first_order, nominal] =
        high_drive_voltages.expect("high-drive sample was recorded");
    assert!((full - explicit).abs() < 1e-9);
    assert!(
        (full - first_order).abs() > 0.01,
        "TIS2 must change the transport-current load line: {full} V vs {first_order} V"
    );
    assert!(
        (full - nominal).abs() > 0.005,
        "hot TIS coefficients must change the load line from nominal IS: {full} V vs {nominal} V"
    );
}

#[test]
fn nrf5340dk_bjt_tlev_zero_and_one_beta_temperature_laws_reach_saadc() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let tlev = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-tlev-temperature-system.yaml",
    ))
    .expect("load TLEV=0/1 beta-temperature branches and hot equivalents");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(tlev.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 8);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and six generic BJT TLEV branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve to SAADC AIN0 through AIN7"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_tlev_temperature.forward_base_v", 0.0)
        .expect("initialize forward-active BJT base drive");
    session
        .set_signal_number("ui.bjt_tlev_temperature.reverse_base_v", 0.0)
        .expect("initialize reverse-active BJT base drive");

    const TNOM_K: f64 = 25.0 + 273.15;
    const TEMP_K: f64 = 125.0 + 273.15;
    const TEMP_C: f64 = 125.0;
    const DELTA_C: f64 = TEMP_C - 25.0;
    const IS_NOMINAL_A: f64 = 1e-16;
    const EG_EV: f64 = 1.11;
    const XTI: f64 = 3.0;
    const XTB: f64 = 0.01;
    const BF_NOMINAL: f64 = 80.0;
    const BR_NOMINAL: f64 = 4.0;
    let ratio = TEMP_K / TNOM_K;
    let thermal_v = 1.380_649e-23 * TEMP_K / 1.602_176_634e-19;
    let is_hot_a = IS_NOMINAL_A * (((ratio - 1.0) * EG_EV / thermal_v) + XTI * ratio.ln()).exp();
    let tlev_zero_factor = ratio.powf(XTB);
    let tlev_one_factor = 1.0 + XTB * DELTA_C;
    let bf_tlev_zero = BF_NOMINAL * tlev_zero_factor;
    let br_tlev_zero = BR_NOMINAL * tlev_zero_factor;
    let bf_tlev_one = BF_NOMINAL * tlev_one_factor;
    let br_tlev_one = BR_NOMINAL * tlev_one_factor;
    assert!(
        (bf_tlev_one - bf_tlev_zero).abs() > 79.0,
        "TLEV=0 power law and TLEV=1 linear law must remain distinct"
    );

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_voltages = None;
    for (forward_drive_v, reverse_drive_v) in [(1.0_f64, 0.9_f64), (1.1, 1.0), (1.2, 1.1)] {
        session
            .set_signal_number("ui.bjt_tlev_temperature.forward_base_v", forward_drive_v)
            .expect("set common forward-active base drive");
        session
            .set_signal_number("ui.bjt_tlev_temperature.reverse_base_v", reverse_drive_v)
            .expect("set common reverse-active base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and hot BJT TLEV branches");
        assert_eq!(steps.len(), 8, "all eight generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected SAADC route errors: {errors:?}"
        );

        let routed_ntc_v = voltage(&session, "adc.saadc.0_volts");
        let routed_diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!((routed_ntc_v - ntc_v).abs() < 1e-9);
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the independent Shockley diode remains isolated"
        );
        let actual = [
            voltage(&session, "adc.saadc.2_volts"),
            voltage(&session, "adc.saadc.3_volts"),
            voltage(&session, "adc.saadc.4_volts"),
            voltage(&session, "adc.saadc.5_volts"),
            voltage(&session, "adc.saadc.6_volts"),
            voltage(&session, "adc.saadc.7_volts"),
        ];
        let expected = [
            bjt_hot_load_line_voltage(forward_drive_v, is_hot_a, bf_tlev_zero, br_tlev_zero, false),
            bjt_hot_load_line_voltage(forward_drive_v, is_hot_a, bf_tlev_zero, br_tlev_zero, false),
            bjt_hot_load_line_voltage(forward_drive_v, is_hot_a, bf_tlev_one, br_tlev_one, false),
            bjt_hot_load_line_voltage(forward_drive_v, is_hot_a, bf_tlev_one, br_tlev_one, false),
            bjt_hot_load_line_voltage(reverse_drive_v, is_hot_a, bf_tlev_zero, br_tlev_zero, true),
            bjt_hot_load_line_voltage(reverse_drive_v, is_hot_a, bf_tlev_one, br_tlev_one, true),
        ];
        for (index, (actual_v, expected_v)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                (actual_v - expected_v).abs() < 5e-5,
                "SAADC AIN{} independent TLEV beta load line should be {expected_v} V, got {actual_v} V",
                index + 2
            );
        }
        assert!(
            (actual[0] - actual[1]).abs() < 1e-9,
            "TLEV=0 must match its explicit hot IS/BF/BR card"
        );
        assert!(
            (actual[2] - actual[3]).abs() < 1e-9,
            "TLEV=1 must match its explicit hot IS/BF/BR card"
        );

        let codes = scan_saadc_channels::<8>(&mut bus);
        for (channel, voltage_v) in [
            (0, routed_ntc_v),
            (1, routed_diode_v),
            (2, actual[0]),
            (3, actual[1]),
            (4, actual[2]),
            (5, actual[3]),
            (6, actual[4]),
            (7, actual[5]),
        ] {
            assert_eq!(
                codes[channel],
                expected_saadc_code(voltage_v),
                "SAADC AIN{channel} converts its routed solved voltage"
            );
        }
        if forward_drive_v == 1.2 {
            high_drive_voltages = Some(actual);
        }
    }

    let [tlev0, tlev0_explicit, tlev1, tlev1_explicit, reverse_tlev0, reverse_tlev1] =
        high_drive_voltages.expect("high-drive sample was recorded");
    assert!((tlev0 - tlev0_explicit).abs() < 1e-9);
    assert!((tlev1 - tlev1_explicit).abs() < 1e-9);
    assert!(
        (tlev0 - tlev1).abs() > 0.01,
        "TLEV selector must change forward beta load line: {tlev0} V vs {tlev1} V"
    );
    assert!(
        (reverse_tlev0 - reverse_tlev1).abs() > 0.01,
        "TLEV selector must change reverse-active beta load line: {reverse_tlev0} V vs {reverse_tlev1} V"
    );
}

#[test]
fn nrf5340dk_bjt_itf_temperature_law_reaches_saadc_during_transit_charge() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let transit = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-itf-temperature-system.yaml",
    ))
    .expect("load generic BJT TITF branch and explicit hot references");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(transit.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 8);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and six generic BJT transit branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve to SAADC AIN0 through AIN7"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_itf_temperature.base_v", 0.0)
        .expect("initialize all matched BJT base drives");

    const DELTA_C: f64 = 80.0 - 27.0;
    const ITF_NOMINAL_A: f64 = 2e-6;
    const TITF1_PER_C: f64 = -8e-3;
    const TITF2_PER_C2: f64 = 200e-6;
    let itf_first_order_a = ITF_NOMINAL_A * (1.0 + TITF1_PER_C * DELTA_C);
    let itf_quadratic_a =
        ITF_NOMINAL_A * (1.0 + TITF1_PER_C * DELTA_C + TITF2_PER_C2 * DELTA_C * DELTA_C);
    assert!((itf_first_order_a - 1.152e-6).abs() < 1e-18);
    assert!((itf_quadratic_a - 2.2756e-6).abs() < 1e-18);

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for boundary in 0..100 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("initialize NTC, diode and matched BJT branches at zero drive");
        let expected_steps = if boundary == 99 { 8 } else { 6 };
        assert_eq!(
            steps.len(),
            expected_steps,
            "the six 10 ns BJT branches and 1 us generic routes advance at their own periods"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");
    }

    session
        .set_signal_number("ui.bjt_itf_temperature.base_v", 0.65)
        .expect("step every BJT base to charge its forward transit capacitance");
    let mut first_voltages = None;
    let mut settled_voltages = None;
    for step in 0..1_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode and BJT transit-charge branches");
        let expected_steps = if step % 100 == 99 { 8 } else { 6 };
        assert_eq!(
            steps.len(),
            expected_steps,
            "six BJT branches solve each 10 ns step; NTC and diode refresh each microsecond"
        );
        assert!(
            errors.is_empty(),
            "unexpected SAADC route errors: {errors:?}"
        );

        let routed_ntc_v = voltage(&session, "adc.saadc.0_volts");
        let routed_diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!(
            (routed_ntc_v - ntc_v).abs() < 1e-9,
            "NTC route stays isolated"
        );
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains isolated"
        );
        let actual = [
            voltage(&session, "adc.saadc.2_volts"),
            voltage(&session, "adc.saadc.3_volts"),
            voltage(&session, "adc.saadc.4_volts"),
            voltage(&session, "adc.saadc.5_volts"),
            voltage(&session, "adc.saadc.6_volts"),
            voltage(&session, "adc.saadc.7_volts"),
        ];
        if step == 0 {
            assert!(
                (actual[0] - actual[1]).abs() < 1e-8,
                "TITF hot branch must match its explicit hot-ITF card: {actual:?}"
            );
            assert!(
                actual[0] > actual[2] + 1e-3,
                "reduced hot ITF increases transit charge and raises the loaded collector: {actual:?}"
            );
            assert!(
                (actual[0] - actual[3]).abs() > 1e-3,
                "the temperature-adjusted ITF must differ from the ITF=0 control: {actual:?}"
            );
            assert!(
                actual[3] > actual[2] + 1e-3,
                "ITF=0 removes current suppression and changes the transient: {actual:?}"
            );
            assert!(
                (actual[4] - actual[5]).abs() < 1e-8,
                "quadratic TITF branch must match its explicit hot-ITF equivalent: {actual:?}"
            );
            assert!(
                actual[0] > actual[4] + 1e-3,
                "TITF2 must change transit charge from the first-order TITF1 result: {actual:?}"
            );
            first_voltages = Some(actual);
        }
        if step == 999 {
            settled_voltages = Some(actual);
        }

        if step == 0 || step == 999 {
            let codes = scan_saadc_channels::<8>(&mut bus);
            for (channel, voltage_v) in [
                (0, routed_ntc_v),
                (1, routed_diode_v),
                (2, actual[0]),
                (3, actual[1]),
                (4, actual[2]),
                (5, actual[3]),
                (6, actual[4]),
                (7, actual[5]),
            ] {
                assert_eq!(
                    codes[channel],
                    expected_saadc_code(voltage_v),
                    "SAADC AIN{channel} converts its routed solved voltage at step {step}"
                );
            }
            if step == 0 {
                assert_ne!(
                    codes[2], codes[6],
                    "TITF2 must change the firmware-visible SAADC code from the first-order TITF1 result"
                );
            }
        }
    }

    let first = first_voltages.expect("first transit-charge sample exists");
    let settled = settled_voltages.expect("settled transit-charge sample exists");
    for (label, voltage_v) in [
        ("TITF", settled[0]),
        ("explicit hot ITF", settled[1]),
        ("ITF=0", settled[3]),
        ("quadratic TITF", settled[4]),
        ("explicit hot quadratic ITF", settled[5]),
    ] {
        assert!(
            (voltage_v - settled[2]).abs() < 1e-3,
            "TITF changes transit charge only; {label} and nominal ITF settle to one DC point: {settled:?}"
        );
    }
    assert!(
        (first[0] - first[2]).abs() > 1e-3,
        "TITF must change the first transient before it decays: {first:?}"
    );
}
