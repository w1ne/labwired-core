// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through every exact-part STM32 catalog profile
//! (`configs/systems/stm32-catalog/*.yaml`) into ADC1.
//!
//! Nothing here is per-part: the ADC1 base, register profile, clock gate and
//! the package's highest-numbered bonded ADC pad all come from each part's chip
//! descriptor. A new catalog profile is covered by this test the moment it
//! lands, and a part whose descriptor routes a pad wrongly fails here.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::{Path, PathBuf};

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

/// Independent level-1 NMOS load-line reference with optional model sheet
/// resistance multiplied by the instance's drain/source square counts.
fn mosfet_drain_voltage(gate_v: f64, drain_resistance: f64, source_resistance: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 10_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;

    let beta = KP_A_PER_V2 * W_M / L_M;
    let current_at = |current: f64| {
        let source_internal_v = current * source_resistance;
        let drain_internal_v = SUPPLY_V - current * (LOAD_OHMS + drain_resistance);
        let vgs = gate_v - source_internal_v;
        let vds = (drain_internal_v - source_internal_v).max(0.0);
        let overdrive = vgs - VTO;
        if overdrive <= 0.0 {
            0.0
        } else if vds < overdrive {
            beta * (1.0 + LAMBDA_PER_V * vds) * vds * (overdrive - 0.5 * vds)
        } else {
            0.5 * beta * (1.0 + LAMBDA_PER_V * vds) * overdrive * overdrive
        }
    };
    let (mut low, mut high) = (
        0.0,
        SUPPLY_V / (LOAD_OHMS + drain_resistance + source_resistance),
    );
    for _ in 0..100 {
        let current = 0.5 * (low + high);
        if current_at(current) > current {
            low = current;
        } else {
            high = current;
        }
    }
    let current = 0.5 * (low + high);
    SUPPLY_V - current * LOAD_OHMS
}

/// Independent Gummel–Poon load line. Each matched branch adds one optional
/// base-junction recombination path (`ISE`/`NE` or `ISC`/`NC`).
fn bjt_collector_voltage(
    base_drive_v: f64,
    base_emitter_recombination: bool,
    base_collector_recombination: bool,
    forward_early_voltage: Option<f64>,
) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
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
        let base_width = 1.0 - forward_early_voltage.map_or(0.0, |vaf| vbc / vaf) - vbe / VAR;
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
        if (base_drive_v - vbe) / BASE_OHMS > base_current {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

fn catalog_profiles() -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root("configs/systems/stm32-catalog"))
        .expect("read STM32 catalog systems")
        .map(|entry| entry.expect("catalog entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    paths
}

/// ADC1 register map and clock gate, read from the part's descriptor.
struct Adc1 {
    base: u64,
    rcc_apb2enr: u64,
    clock_bit: u8,
    f1_profile: bool,
}

impl Adc1 {
    fn from_chip(chip: &ChipDescriptor) -> Self {
        let adc = chip
            .peripherals
            .iter()
            .find(|p| p.id == "adc1")
            .unwrap_or_else(|| panic!("{} declares ADC1", chip.name));
        let profile = adc.config["profile"]
            .as_str()
            .expect("ADC1 names a profile");
        let f1_profile = match profile {
            "stm32f1" => true,
            "stm32f4" => false,
            other => panic!("{}: unexpected ADC1 profile {other}", chip.name),
        };
        let gate = &adc.clock.as_ref().expect("ADC1 is clock gated").as_slice()[0];
        assert_eq!(gate.reg, "apb2enr", "{}: ADC1 sits on APB2", chip.name);
        let rcc = chip
            .peripherals
            .iter()
            .find(|p| p.id == "rcc")
            .expect("RCC present");
        // RM0008 §7.3.7 (APB2ENR @ 0x18) vs RM0090/RM0368 §6.3 (APB2ENR @ 0x44).
        let apb2enr_offset = if f1_profile { 0x18 } else { 0x44 };
        Self {
            base: adc.base_address,
            rcc_apb2enr: rcc.base_address + apb2enr_offset,
            clock_bit: gate.bit,
            f1_profile,
        }
    }

    fn channel_count(&self, bus: &mut SystemBus, channel: u8) -> u16 {
        let index = bus
            .find_peripheral_index_by_name("adc1")
            .expect("ADC1 on bus");
        bus.peripherals[index]
            .dev
            .as_any_mut()
            .expect("ADC1 downcasts")
            .downcast_mut::<labwired_core::peripherals::adc::Adc>()
            .expect("ADC1 uses the shared STM32 ADC model")
            .channel_input_count(channel)
    }

    /// One-shot software conversion through the register sequence firmware uses.
    fn convert(&self, bus: &mut SystemBus, channel: u8) -> u32 {
        const ADON: u32 = 1;
        const EOC: u32 = 1 << 1;
        let apb2enr = bus.read_u32(self.rcc_apb2enr).expect("read RCC APB2ENR");
        bus.write_u32(self.rcc_apb2enr, apb2enr | (1 << self.clock_bit))
            .expect("enable ADC1 clock");
        bus.write_u32(self.base + 0x34, u32::from(channel))
            .expect("select SQR3 SQ1 channel");
        let (armed, start) = if self.f1_profile {
            // F1: EXTSEL=SWSTART, EXTTRIG, then SWSTART is CR2 bit 22.
            let armed = ADON | (0b111 << 17) | (1 << 20);
            (armed, armed | (1 << 22))
        } else {
            // F4: SWSTART is CR2 bit 30.
            (ADON, ADON | (1 << 30))
        };
        bus.write_u32(self.base + 0x08, armed)
            .expect("power up ADC1");
        bus.write_u32(self.base + 0x08, start)
            .expect("software-start ADC1 conversion");
        for _ in 0..64 {
            bus.tick_peripherals_fully();
        }
        let status = bus.read_u32(self.base).expect("read ADC1 SR");
        assert_ne!(status & EOC, 0, "ADC1 conversion completes with EOC set");
        bus.read_u32(self.base + 0x4c).expect("read ADC1 DR")
    }

    /// One software-started `CR1.SCAN` group over `channels` (SQ1.. in
    /// SQR3, `L` in SQR1[23:20]), read the way polling firmware without DMA
    /// can: on the F4 with `CR2.EOCS` set, wait for `EOC`, clear it and read
    /// `DR` once per channel; on the F1, which raises `EOC` only at the end
    /// of the group (RM0008 §11.3.8), read the group's last result. Returns
    /// the codes read and the ticks until the first `EOC`. Leaves CR1/SQR1
    /// at the single-conversion reset state.
    fn scan(&self, bus: &mut SystemBus, channels: &[u8]) -> (Vec<u32>, usize) {
        const ADON: u32 = 1;
        const EOC: u32 = 1 << 1;
        const SCAN: u32 = 1 << 8;
        const EOCS: u32 = 1 << 10;
        let sqr3 = channels
            .iter()
            .enumerate()
            .fold(0, |w, (k, &ch)| w | (u32::from(ch) << (5 * k)));
        bus.write_u32(self.base + 0x34, sqr3).expect("SQR3 SQ1..");
        bus.write_u32(self.base + 0x2c, (channels.len() as u32 - 1) << 20)
            .expect("SQR1.L");
        bus.write_u32(self.base + 0x04, SCAN).expect("CR1.SCAN");
        bus.write_u32(self.base, 0).expect("clear SR");
        let armed = if self.f1_profile {
            ADON | (0b111 << 17) | (1 << 20)
        } else {
            ADON | EOCS
        };
        let start = armed | if self.f1_profile { 1 << 22 } else { 1 << 30 };
        bus.write_u32(self.base + 0x08, armed).expect("arm ADC1");
        bus.write_u32(self.base + 0x08, start).expect("SWSTART");
        let mut first_eoc = None;
        let mut codes = Vec::new();
        let reads = if self.f1_profile { 1 } else { channels.len() };
        let mut ticks = 0;
        while codes.len() < reads {
            assert!(ticks < 64 * channels.len(), "ADC1 scan stalls");
            bus.tick_peripherals_fully();
            ticks += 1;
            if bus.read_u32(self.base).unwrap() & EOC != 0 {
                first_eoc.get_or_insert(ticks);
                bus.write_u32(self.base, 0).expect("clear EOC");
                codes.push(bus.read_u32(self.base + 0x4c).expect("read DR"));
            }
        }
        bus.write_u32(self.base + 0x04, 0).expect("restore CR1");
        bus.write_u32(self.base + 0x2c, 0).expect("restore SQR1");
        (codes, first_eoc.unwrap())
    }

    /// `CR1.DISCEN` over `channels` with `DISCNUM` = `per_trigger` - 1: each
    /// SWSTART converts the next `per_trigger` channels (RM0008 §11.3.10,
    /// RM0090 §13.3.10). Ticks each trigger for exactly its subgroup's
    /// conversion time, then twice as long again to prove the subgroup
    /// stopped. Returns `(DR, EOC)` per trigger. `EOC` (with `EOCS` clear)
    /// must stay low until the subgroup reaching SQ(L+1) lands. Leaves
    /// CR1/SQR1 at the single-conversion reset state.
    fn discontinuous(
        &self,
        bus: &mut SystemBus,
        channels: &[u8],
        per_trigger: usize,
        triggers: usize,
        single_ticks: usize,
    ) -> Vec<(u32, bool)> {
        const ADON: u32 = 1;
        const EOC: u32 = 1 << 1;
        const SCAN: u32 = 1 << 8;
        const DISCEN: u32 = 1 << 11;
        let sqr3 = channels
            .iter()
            .enumerate()
            .fold(0, |w, (k, &ch)| w | (u32::from(ch) << (5 * k)));
        bus.write_u32(self.base + 0x34, sqr3).expect("SQR3 SQ1..");
        bus.write_u32(self.base + 0x2c, (channels.len() as u32 - 1) << 20)
            .expect("SQR1.L");
        let discnum = (per_trigger as u32 - 1) << 13;
        bus.write_u32(self.base + 0x04, SCAN | DISCEN | discnum)
            .expect("CR1.DISCEN");
        let armed = if self.f1_profile {
            ADON | (0b111 << 17) | (1 << 20)
        } else {
            ADON
        };
        let start = armed | if self.f1_profile { 1 << 22 } else { 1 << 30 };
        bus.write_u32(self.base + 0x08, armed).expect("arm ADC1");
        let mut next = 0;
        let out = (0..triggers)
            .map(|t| {
                bus.write_u32(self.base, 0).expect("clear SR");
                bus.write_u32(self.base + 0x08, start).expect("SWSTART");
                let n = per_trigger.min(channels.len() - next);
                next = (next + n) % channels.len();
                for tick in 1..=n * single_ticks {
                    bus.tick_peripherals_fully();
                    if tick < n * single_ticks {
                        assert_eq!(
                            bus.read_u32(self.base).unwrap() & EOC,
                            0,
                            "ADC1 trigger {t}: EOC before its subgroup lands"
                        );
                    }
                }
                let dr = bus.read_u32(self.base + 0x4c).expect("read DR");
                let eoc = bus.read_u32(self.base).unwrap() & EOC != 0;
                for _ in 0..2 * single_ticks {
                    bus.tick_peripherals_fully();
                }
                assert_eq!(
                    bus.read_u32(self.base + 0x4c).unwrap(),
                    dr,
                    "ADC1 trigger {t}: the subgroup stops after {n} conversions"
                );
                (dr, eoc)
            })
            .collect();
        bus.write_u32(self.base + 0x04, 0).expect("restore CR1");
        bus.write_u32(self.base + 0x2c, 0).expect("restore SQR1");
        out
    }

    /// One software-started injected group over `channels` (rank 1 first)
    /// with `JOFRx` = `offsets`, programmed the way the ST HAL's
    /// `ADC_JSQR_RK_JL` does: rank r of an n-channel group sits in slot
    /// `JSQ(4 - n + r)`. F1: `JEXTSEL` = 111 and `JSWSTART` is CR2 bit 21;
    /// F2/F4: `JSWSTART` is CR2 bit 22 (RM0008 §11.12.3, RM0090 §13.13.3).
    /// Returns `JDR1..JDRn` (sign-extended 16-bit) and the ticks to `JEOC`.
    fn injected(&self, bus: &mut SystemBus, channels: &[u8], offsets: &[u32]) -> (Vec<u32>, usize) {
        const ADON: u32 = 1;
        const JEOC: u32 = 1 << 2;
        let n = channels.len();
        let jsqr = channels
            .iter()
            .enumerate()
            .fold(((n as u32) - 1) << 20, |w, (r, &ch)| {
                w | (u32::from(ch) << (5 * (4 - n + r)))
            });
        bus.write_u32(self.base + 0x38, jsqr).expect("JSQR");
        for (k, offset) in offsets.iter().enumerate() {
            bus.write_u32(self.base + 0x14 + 4 * k as u64, *offset)
                .expect("JOFRx");
        }
        bus.write_u32(self.base, 0).expect("clear SR");
        let (armed, start) = if self.f1_profile {
            let armed = ADON | (0b111 << 12) | (1 << 15);
            (armed, armed | (1 << 21))
        } else {
            (ADON, ADON | (1 << 22))
        };
        bus.write_u32(self.base + 0x08, armed).expect("arm ADC1");
        bus.write_u32(self.base + 0x08, start).expect("JSWSTART");
        let mut ticks = 0;
        while bus.read_u32(self.base).unwrap() & JEOC == 0 {
            assert!(ticks < 64 * n, "ADC1 injected group stalls");
            bus.tick_peripherals_fully();
            ticks += 1;
        }
        let jdr = (0..n)
            .map(|k| bus.read_u32(self.base + 0x3c + 4 * k as u64).expect("JDRx"))
            .collect();
        for k in 0..offsets.len() {
            bus.write_u32(self.base + 0x14 + 4 * k as u64, 0)
                .expect("restore JOFRx");
        }
        (jdr, ticks)
    }
}

/// The highest-channel ADC1 pad this package bonds out — PB1 on 36/48-pin
/// parts, PC5 on 64-pin and larger parts.
fn top_adc1_pad(chip: &ChipDescriptor) -> (String, u8) {
    chip.analog_pins
        .iter()
        .filter(|(_, route)| route.peripheral == "adc1")
        .max_by_key(|(_, route)| route.channel)
        .map(|(pad, route)| (pad.clone(), route.channel))
        .unwrap_or_else(|| panic!("{} routes no ADC1 pads", chip.name))
}

fn generic_netlists(diode_pad: &str) -> SystemManifest {
    let mut analog = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    let diode = analog
        .cosim_models
        .iter_mut()
        .find(|model| model.id == "diode_load_line")
        .expect("fixture carries the diode load line");
    diode.outputs.insert(
        "v_diode".into(),
        format!("board.analog.{}_volts", diode_pad.to_ascii_lowercase()),
    );
    let recombination =
        SystemManifest::from_file(root("examples/stm32-catalog/analog-bjt-recombination.yaml"))
            .expect("load matched generic BJT recombination branches");
    analog.cosim_models.extend(recombination.cosim_models);
    analog
}

fn mosfet_sheet_resistance_netlists() -> SystemManifest {
    SystemManifest::from_file(root(
        "examples/stm32-catalog/analog-mosfet-sheet-resistance.yaml",
    ))
    .expect("load generic MOSFET sheet-resistance branches")
}

fn assert_catalog_profile_routes_generic_analog(system_path: &Path) {
    let mut profile = SystemManifest::from_file(system_path)
        .unwrap_or_else(|e| panic!("load {}: {e}", system_path.display()));
    assert!(profile.external_devices.is_empty() && profile.board_io.is_empty());
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load catalog part descriptor");
    let part = chip.name.clone();
    assert_eq!(profile.name, format!("{part}-smoke"));

    let adc1 = Adc1::from_chip(&chip);
    assert_eq!(
        chip.analog_pins["PA0"].channel, 0,
        "{part}: PA0 is ADC1_IN0"
    );
    for (pad, channel) in [("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{part}: {pad} is ADC1_IN{channel}"
        );
        assert_eq!(
            chip.analog_pins[pad].peripheral, "adc1",
            "{part}: {pad} routes to ADC1"
        );
    }
    let (top_pad, top_channel) = top_adc1_pad(&chip);
    let small_package = !chip.analog_pins.contains_key("PC0");
    let expected_top = if small_package {
        ("PB1", 9)
    } else {
        ("PC5", 15)
    };
    assert_eq!(
        (top_pad.as_str(), top_channel),
        expected_top,
        "{part}: package's highest ADC1 pad"
    );

    profile.cosim_models = generic_netlists(&top_pad).cosim_models;
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build catalog part bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{part}: generic netlists resolve through PA0/PA1/PA2/PA3/{top_pad}"
    );
    let idle_channel = 4;
    for channel in [0, 1, 2, 3, idle_channel, top_channel] {
        assert_eq!(
            adc1.channel_count(&mut bus, channel),
            0xFFFF,
            "{part}: ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    let diode_path = format!("board.analog.{}_volts", top_pad.to_ascii_lowercase());
    let mut cycles = 0;
    let mut ntc_counts = Vec::new();
    session
        .set_signal_number("ui.bjt.base_v", 0.8)
        .expect("set generic BJT base drive");
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all generic analog netlists");
        assert_eq!(
            steps.len(),
            5,
            "{part}: NTC, diode, and all three BJT branches cross the 1 us boundary"
        );
        assert!(
            errors.is_empty(),
            "{part}: unexpected route errors: {errors:?}"
        );

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{part}: at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, &diode_path);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{part}: Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        let bjt_reference_v = signal_voltage(&session, "board.analog.pa1_volts");
        let bjt_recombination_v = signal_voltage(&session, "board.analog.pa2_volts");
        let expected_bjt_reference_v = bjt_collector_voltage(0.8, false, false, Some(50.0));
        let expected_bjt_recombination_v = bjt_collector_voltage(0.8, true, false, Some(50.0));
        assert!(
            (bjt_reference_v - expected_bjt_reference_v).abs() < 5e-5,
            "{part}: BJT without recombination should be {expected_bjt_reference_v} V, got {bjt_reference_v} V"
        );
        assert!(
            (bjt_recombination_v - expected_bjt_recombination_v).abs() < 5e-5,
            "{part}: BJT with ISE/NE recombination should be {expected_bjt_recombination_v} V, got {bjt_recombination_v} V"
        );
        assert!(
            bjt_recombination_v > bjt_reference_v + 0.005,
            "{part}: recombination current loads the base drive and lowers collector current"
        );
        let bjt_base_collector_recombination_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_bjt_base_collector_recombination_v =
            bjt_collector_voltage(0.8, false, true, Some(50.0));
        assert!(
            (bjt_base_collector_recombination_v
                - expected_bjt_base_collector_recombination_v)
                .abs()
                < 5e-5,
            "{part}: BJT with ISC/NC recombination should be {expected_bjt_base_collector_recombination_v} V, got {bjt_base_collector_recombination_v} V"
        );

        for (channel, volts) in [
            (0, ntc_v),
            (1, bjt_reference_v),
            (2, bjt_recombination_v),
            (3, bjt_base_collector_recombination_v),
            (top_channel, diode_v),
        ] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc1.channel_count(&mut bus, channel),
                expected_count,
                "{part}: solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                adc1.convert(&mut bus, channel),
                u32::from(expected_count),
                "{part}: firmware-visible ADC1 channel {channel} conversion"
            );
        }
        assert_eq!(
            adc1.channel_count(&mut bus, idle_channel),
            0xFFFF,
            "{part}: unrouted ADC1 channel {idle_channel} stays undriven"
        );
        ntc_counts.push(adc1.channel_count(&mut bus, 0));

        // One trigger scans both routed sensors through CR1.SCAN, in either
        // SQx order, and lands the same codes as the single conversions.
        let ntc = u32::from(expected_adc_count(ntc_v));
        let diode = u32::from(expected_adc_count(diode_v));
        let (_, single_ticks) = adc1.scan(&mut bus, &[0]);
        for (order, codes) in [
            (vec![0, top_channel], vec![ntc, diode]),
            (vec![top_channel, 0, top_channel], vec![diode, ntc, diode]),
        ] {
            let (read, first_eoc) = adc1.scan(&mut bus, &order);
            if adc1.f1_profile {
                assert_eq!(
                    first_eoc,
                    order.len() * single_ticks,
                    "{part}: F1 raises EOC once, after all {} conversions",
                    order.len()
                );
                assert_eq!(
                    read,
                    vec![*codes.last().unwrap()],
                    "{part}: F1 DR holds the group's last channel {order:?}"
                );
            } else {
                assert_eq!(
                    first_eoc, single_ticks,
                    "{part}: EOCS raises EOC per conversion"
                );
                assert_eq!(read, codes, "{part}: F4 scan {order:?} polled per EOC");
            }
        }

        // CR1.DISCEN, two channels per trigger over [NTC, diode, NTC]:
        // [NTC, diode], then [NTC] reaching SQ3 with EOC, then SQ1 again.
        assert_eq!(
            adc1.discontinuous(&mut bus, &[0, top_channel, 0], 2, 3, single_ticks),
            vec![(diode, false), (ntc, true), (diode, false)],
            "{part}: DISCEN steps the routed sensors in DISCNUM+1 subgroups"
        );
        // DISCNUM = 0: one sensor per trigger, the polling-firmware pattern.
        assert_eq!(
            adc1.discontinuous(&mut bus, &[top_channel, 0], 1, 3, single_ticks),
            vec![(diode, false), (ntc, true), (diode, false)],
            "{part}: DISCEN converts one routed sensor per trigger"
        );

        // Injected group: both routed sensors convert on one JSWSTART into
        // JDR1..JDRn in rank order, with no regular group touched.
        let dr_before = bus.read_u32(adc1.base + 0x4c).unwrap();
        for (ranks, codes) in [
            (vec![top_channel], vec![diode]),
            (vec![0, top_channel], vec![ntc, diode]),
            (
                vec![top_channel, 0, idle_channel, top_channel],
                vec![diode, ntc, 0, diode],
            ),
        ] {
            let (jdr, ticks) = adc1.injected(&mut bus, &ranks, &[]);
            assert_eq!(
                jdr, codes,
                "{part}: injected ranks {ranks:?} land in JDR order"
            );
            assert_eq!(
                ticks,
                ranks.len() * single_ticks,
                "{part}: JEOC after all {} injected conversions",
                ranks.len()
            );
        }
        // JOFRx subtracts a per-rank offset; the diode's code (~0.6 V) less
        // the NTC's offset goes negative and reads back sign-extended.
        let (jdr, _) = adc1.injected(&mut bus, &[0, top_channel], &[ntc, 2047]);
        assert_eq!(
            jdr,
            vec![0, u32::from((diode as i32 - 2047) as i16 as u16)],
            "{part}: JOFRx offsets the routed injected results"
        );
        assert_eq!(
            bus.read_u32(adc1.base + 0x4c).unwrap(),
            dr_before,
            "{part}: the injected group leaves the regular DR alone"
        );
    }
    assert!(
        ntc_counts[0] < ntc_counts[1] && ntc_counts[1] < ntc_counts[2],
        "{part}: ADC counts track the NTC temperature: {ntc_counts:?}"
    );
    assert_eq!(
        ntc_counts[1], 2047,
        "{part}: 25 °C NTC divider is half scale"
    );

    // Sweep the BJT stimulus at a fixed NTC temperature. The independent
    // diode and NTC retain their own solutions while recombination changes
    // the base current and therefore the BJT collector load line.
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold unrelated NTC at nominal temperature");
    for base_drive_v in [0.7_f64, 0.9, 1.2] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("step matched BJT base drives");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all generic analog netlists");
        assert_eq!(steps.len(), 5, "{part}: all five analog netlists solve");
        assert!(
            errors.is_empty(),
            "{part}: unexpected route errors: {errors:?}"
        );

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, &diode_path);
        let reference_v = signal_voltage(&session, "board.analog.pa1_volts");
        let recombination_v = signal_voltage(&session, "board.analog.pa2_volts");
        let base_collector_recombination_v = signal_voltage(&session, "board.analog.pa3_volts");
        assert!((ntc_v - 1.65).abs() < 1e-9, "{part}: NTC remains isolated");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{part}: unrelated diode remains isolated"
        );
        assert!(
            (reference_v - bjt_collector_voltage(base_drive_v, false, false, Some(50.0))).abs()
                < 5e-5,
            "{part}: reference BJT follows its independent load line at {base_drive_v} V"
        );
        assert!(
            (recombination_v - bjt_collector_voltage(base_drive_v, true, false, Some(50.0))).abs()
                < 5e-5,
            "{part}: recombination BJT follows its independent load line at {base_drive_v} V"
        );
        assert!(
            recombination_v > reference_v + 0.005,
            "{part}: recombination changes the BJT collector load line at {base_drive_v} V"
        );
        let expected_base_collector_v =
            bjt_collector_voltage(base_drive_v, false, true, Some(50.0));
        assert!(
            (base_collector_recombination_v - expected_base_collector_v).abs() < 5e-5,
            "{part}: ISC/NC recombination predicts {expected_base_collector_v} V at {base_drive_v} V, got {base_collector_recombination_v} V"
        );
        if base_drive_v >= 1.2 {
            assert!(
                base_collector_recombination_v > reference_v + 0.1,
                "{part}: forward-biased base-collector recombination must load the collector at {base_drive_v} V"
            );
        }
        for (channel, volts) in [
            (1, reference_v),
            (2, recombination_v),
            (3, base_collector_recombination_v),
        ] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc1.channel_count(&mut bus, channel),
                expected,
                "{part}: BJT voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                adc1.convert(&mut bus, channel),
                u32::from(expected),
                "{part}: firmware-visible BJT conversion on ADC1 channel {channel}"
            );
        }
    }

    // A pad the package does not bond to an ADC input must be refused at
    // binding time, not silently land on some other channel.
    if small_package {
        let rejected = generic_netlists("PC0");
        let session = CosimSession::new(&rejected.cosim_models, Path::new("."), &bus)
            .expect("build analog co-simulation")
            .expect("profile carries the generic analog devices");
        assert!(
            session
                .binding_errors()
                .iter()
                .any(|e| format!("{e:?}").contains("board.analog.pc0_volts")),
            "{part}: PC0 has no ADC route on this package, got {:?}",
            session.binding_errors()
        );
    }
}

#[test]
fn every_stm32_catalog_profile_routes_ntc_diode_and_bjt_recombination_into_adc1() {
    let profiles = catalog_profiles();
    assert_eq!(
        profiles.len(),
        26,
        "catalog profile count changed; update the coverage doc"
    );
    for system_path in &profiles {
        assert_catalog_profile_routes_generic_analog(system_path);
    }
}

#[test]
fn every_stm32_catalog_profile_routes_mosfet_sheet_resistance_into_adc1() {
    let profiles = catalog_profiles();
    assert_eq!(
        profiles.len(),
        26,
        "catalog profile count changed; update the coverage doc"
    );
    for system_path in profiles {
        let mut profile = SystemManifest::from_file(&system_path)
            .unwrap_or_else(|e| panic!("load {}: {e}", system_path.display()));
        let chip_path = system_path.parent().unwrap().join(&profile.chip);
        let chip = ChipDescriptor::from_file(&chip_path).expect("load catalog part descriptor");
        let part = chip.name.clone();
        let adc1 = Adc1::from_chip(&chip);
        for (pad, channel) in [("PA4", 4), ("PA5", 5), ("PA6", 6)] {
            assert_eq!(
                chip.analog_pins[pad].channel, channel,
                "{part}: {pad} is ADC1_IN{channel}"
            );
            assert_eq!(
                chip.analog_pins[pad].peripheral, "adc1",
                "{part}: {pad} routes to ADC1"
            );
        }
        let (top_pad, top_channel) = top_adc1_pad(&chip);
        let mut analog = generic_netlists(&top_pad);
        analog
            .cosim_models
            .extend(mosfet_sheet_resistance_netlists().cosim_models);
        profile.cosim_models = analog.cosim_models;

        let mut bus = SystemBus::from_config(&chip, &profile).expect("build catalog part bus");
        let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
            .expect("build analog co-simulation")
            .expect("profile carries NTC, diode, BJT, and MOSFET devices");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "{part}: generic MOSFET branches resolve through PA4/PA5/PA6"
        );
        for channel in [0, 1, 2, 3, 4, 5, 6, top_channel] {
            assert_eq!(
                adc1.channel_count(&mut bus, channel),
                0xFFFF,
                "{part}: ADC1 channel {channel} starts without an analog source"
            );
        }

        session
            .set_signal_number("ui.ntc.temperature_c", 25.0)
            .expect("hold unrelated NTC at nominal temperature");
        let mut cycles = 0;
        for gate_v in [0.5_f64, 2.5] {
            session
                .set_signal_number("ui.mosfet.gate_v", gate_v)
                .expect("set shared generic MOSFET gate drive");
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .expect("solve and route all generic analog netlists");
            assert_eq!(
                steps.len(),
                8,
                "{part}: NTC, diode, three BJT, and three MOSFET branches solve"
            );
            assert!(
                errors.is_empty(),
                "{part}: unexpected route errors: {errors:?}"
            );

            let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
            let diode_path = format!("board.analog.{}_volts", top_pad.to_ascii_lowercase());
            let diode_v = signal_voltage(&session, &diode_path);
            assert!(
                (ntc_v - 1.65).abs() < 1e-9,
                "{part}: unrelated nominal NTC remains isolated at {gate_v} V gate drive"
            );
            assert!(
                (diode_v - diode_load_line_voltage()).abs() < 2e-5,
                "{part}: unrelated Shockley diode remains isolated at {gate_v} V gate drive"
            );

            let expected = [
                ("board.analog.pa4_volts", 0.0, 0.0),
                ("board.analog.pa5_volts", 2.0 * 500.0, 0.0),
                ("board.analog.pa6_volts", 0.0, 4.0 * 500.0),
            ];
            let mut mosfet_voltages = Vec::new();
            for (path, drain_resistance, source_resistance) in expected {
                let volts = signal_voltage(&session, path);
                let reference = mosfet_drain_voltage(gate_v, drain_resistance, source_resistance);
                assert!(
                    (volts - reference).abs() < 5e-5,
                    "{part}: gate {gate_v} V, RD={drain_resistance} Ω, RS={source_resistance} Ω should solve to {reference} V, got {volts} V"
                );
                mosfet_voltages.push(volts);
            }
            if gate_v < 1.0 {
                assert!(
                    mosfet_voltages.iter().all(|v| (v - 3.3).abs() < 5e-5),
                    "{part}: all matched MOSFETs remain cut off below threshold: {mosfet_voltages:?}"
                );
            } else {
                assert!(
                    (mosfet_voltages[1] - mosfet_voltages[0]).abs() > 0.05,
                    "{part}: drain sheet resistance changes the channel load line: {mosfet_voltages:?}"
                );
                assert!(
                    (mosfet_voltages[2] - mosfet_voltages[0]).abs() > 0.1,
                    "{part}: source sheet resistance changes the channel load line: {mosfet_voltages:?}"
                );
            }

            for (channel, volts) in [
                (4, mosfet_voltages[0]),
                (5, mosfet_voltages[1]),
                (6, mosfet_voltages[2]),
            ] {
                let expected_count = expected_adc_count(volts);
                assert_eq!(
                    adc1.channel_count(&mut bus, channel),
                    expected_count,
                    "{part}: solved MOSFET voltage seeds ADC1 channel {channel}"
                );
                assert_eq!(
                    adc1.convert(&mut bus, channel),
                    u32::from(expected_count),
                    "{part}: firmware-visible MOSFET conversion on ADC1 channel {channel}"
                );
            }
        }
    }
}

#[test]
fn every_stm32_catalog_profile_routes_bjt_early_effect_into_adc1_alongside_ntc_and_diode() {
    let profiles = catalog_profiles();
    assert_eq!(
        profiles.len(),
        26,
        "catalog profile count changed; update the coverage doc"
    );

    for system_path in profiles {
        let mut profile = SystemManifest::from_file(&system_path)
            .unwrap_or_else(|e| panic!("load {}: {e}", system_path.display()));
        let chip_path = system_path.parent().unwrap().join(&profile.chip);
        let chip = ChipDescriptor::from_file(&chip_path).expect("load catalog part descriptor");
        let part = chip.name.clone();
        let adc1 = Adc1::from_chip(&chip);
        for (pad, channel) in [("PA4", 4), ("PA5", 5)] {
            assert_eq!(
                chip.analog_pins[pad].channel, channel,
                "{part}: {pad} is ADC1_IN{channel}"
            );
            assert_eq!(
                chip.analog_pins[pad].peripheral, "adc1",
                "{part}: {pad} routes to ADC1"
            );
        }
        let (top_pad, top_channel) = top_adc1_pad(&chip);
        let diode_path = format!("board.analog.{}_volts", top_pad.to_ascii_lowercase());

        let mut analog = SystemManifest::from_file(root(
            "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
        ))
        .expect("load generic Beta NTC divider and Shockley diode");
        let diode = analog
            .cosim_models
            .iter_mut()
            .find(|model| model.id == "diode_load_line")
            .expect("fixture carries the diode load line");
        diode.outputs.insert("v_diode".into(), diode_path.clone());
        let bjt =
            SystemManifest::from_file(root("examples/stm32-catalog/analog-bjt-early-effect.yaml"))
                .expect("load generic matched Gummel–Poon Early-effect branches");
        analog.cosim_models.extend(bjt.cosim_models);
        profile.cosim_models = analog.cosim_models;

        let mut bus = SystemBus::from_config(&chip, &profile).expect("build catalog part bus");
        let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
            .expect("build analog co-simulation")
            .expect("profile carries an NTC, diode, and matched BJT branches");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "{part}: generic BJT branches resolve through PA4/PA5 ADC1"
        );
        for channel in [0, 1, 4, 5, top_channel] {
            assert_eq!(
                adc1.channel_count(&mut bus, channel),
                0xFFFF,
                "{part}: ADC1 channel {channel} starts without an analog source"
            );
        }

        session
            .set_signal_number("ui.ntc.temperature_c", 25.0)
            .expect("hold unrelated NTC at nominal temperature");
        session
            .set_signal_number("ui.bjt_early.base_v", 0.0)
            .expect("initialize matched BJT base drives");
        let expected_diode_v = diode_load_line_voltage();
        let mut cycles = 0;
        let mut early_samples = Vec::new();
        let mut control_samples = Vec::new();
        for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
            session
                .set_signal_number("ui.bjt_early.base_v", base_drive_v)
                .expect("set common BJT base drive");
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .expect("solve and route NTC, diode, and matched BJT branches");
            assert_eq!(steps.len(), 4, "{part}: all four analog branches solve");
            assert!(
                errors.is_empty(),
                "{part}: unexpected analog route errors: {errors:?}"
            );

            let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
            let early_v = signal_voltage(&session, "board.analog.pa4_volts");
            let control_v = signal_voltage(&session, "board.analog.pa5_volts");
            let diode_v = signal_voltage(&session, &diode_path);
            assert!(
                (ntc_v - 1.65).abs() < 1e-9,
                "{part}: unrelated nominal NTC remains isolated at {ntc_v} V"
            );
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{part}: unrelated Shockley diode should be {expected_diode_v} V, got {diode_v} V"
            );

            let expected_early_v = bjt_collector_voltage(base_drive_v, false, false, Some(50.0));
            let expected_control_v = bjt_collector_voltage(base_drive_v, false, false, None);
            assert!(
                (early_v - expected_early_v).abs() < 5e-5,
                "{part}: VAF=50 load line at {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
            );
            assert!(
                (control_v - expected_control_v).abs() < 5e-5,
                "{part}: no-forward-Early load line at {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
            );
            if base_drive_v >= 0.7 {
                assert!(
                    early_v < control_v - 0.001,
                    "{part}: finite VAF increases collector current: {early_v} V vs {control_v} V"
                );
            }

            for (channel, volts) in [
                (0, ntc_v),
                (4, early_v),
                (5, control_v),
                (top_channel, diode_v),
            ] {
                let expected_count = expected_adc_count(volts);
                assert_eq!(
                    adc1.channel_count(&mut bus, channel),
                    expected_count,
                    "{part}: solved voltage seeds ADC1 channel {channel}"
                );
                assert_eq!(
                    adc1.convert(&mut bus, channel),
                    u32::from(expected_count),
                    "{part}: firmware-visible ADC1 conversion on channel {channel}"
                );
            }
            assert_eq!(
                adc1.channel_count(&mut bus, 1),
                0xFFFF,
                "{part}: unrouted ADC1 channel 1 stays isolated"
            );
            early_samples.push(early_v);
            control_samples.push(control_v);
        }

        assert!(
            early_samples[1] > early_samples[2] && early_samples[2] > early_samples[3],
            "{part}: increasing base drive lowers the VAF=50 collector voltage: {early_samples:?}"
        );
        assert!(
            control_samples[1] > control_samples[2] && control_samples[2] > control_samples[3],
            "{part}: increasing base drive lowers the control collector voltage: {control_samples:?}"
        );
    }
}
