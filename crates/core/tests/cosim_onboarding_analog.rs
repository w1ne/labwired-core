// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through every imported onboarding profile
//! (`configs/systems/onboarding/*.yaml`) whose chip descriptor declares an ADC
//! LabWired can convert, into EVERY such converter instance.
//!
//! Nothing here is per-board. The converter family is read from each
//! descriptor (peripheral type plus `config.profile`), the instance base from
//! its `base_address`, and the same two netlists — a Beta NTC divider and an
//! unrelated Shockley diode load line — are routed to `adc.<id>.<ch>_volts`
//! and read back through the register sequence firmware uses for that family.
//! A new onboarding descriptor whose ADC is convertible is covered the moment
//! it lands; one whose ADC changes family moves between the lists below and
//! fails here until the coverage doc is updated.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::{Path, PathBuf};

/// Channels used on every converter: low enough to exist on all four
/// families (nRF52 SAADC has AIN0..7), clear of channel 0.
const NTC_CHANNEL: u8 = 5;
const DIODE_CHANNEL: u8 = 6;
const IDLE_CHANNEL: u8 = 7;

/// Onboarding profiles whose ADC is converted end to end here.
const CONVERTED: &[&str] = &[
    "ambiq-apollo4",
    "cc2538",
    "imxrt1064",
    "mimxrt1064_evk",
    "nrf52840",
    "nucleo_f767zi",
    "nucleo_h753zi",
    "nucleo_wba52cg",
    "ramn",
    "sam4s",
    "sam4s16c",
    "sam4s8b",
    "sam4s_xplained",
    "stm32f042",
    "stm32f072",
    "stm32f072b_discovery",
    "stm32f401cdu6-blackpill",
    "stm32f777",
    "stm32h7",
    "stm32h743",
    "stm32h747-disco",
    "stm32h747",
    "stm32h753",
    "stm32l071",
    "stm32l072",
    "stm32wba52",
    "zolertia-firefly",
];

/// Onboarding profiles that declare an ADC whose IP LabWired does not yet
/// convert from a routed input. Empty: the last two, the STM32WBA52 ADC4
/// boards, now convert on the `stm32wba` profile. A descriptor that lands
/// with an unconvertible ADC fails here until it is converted or listed.
const DECLARED_NOT_CONVERTIBLE: &[&str] = &[];

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

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// The converter families an onboarding descriptor can name.
#[derive(Debug, Clone, Copy)]
enum Family {
    /// Shared `adc` model, F2/F4/F7 map (RM0090/RM0410).
    Stm32F4,
    /// Shared `adc` model, H7 map (RM0433).
    Stm32H7,
    /// Shared `adc` model, L4 map, which the L5 keeps register for register
    /// (RM0351 §18.6, RM0438 §21.6): `SQR1.SQ1` @0x30, `DR` @0x40.
    Stm32L4,
    /// Shared `adc` model, F0/L0 v2 map with the CHSELR bitmap (RM0091 §13,
    /// RM0367 §14).
    Stm32V2,
    /// Shared `adc` model, STM32WBA ADC4 (RM0493 §21): the CHSELR block with
    /// an LDO (`CR.ADVREGEN` → `ISR.LDORDY`) and `CFGR1.RES` at [3:2].
    Stm32Adc4,
    /// i.MX RT 12-bit SAR ADC (IMXRT1060RM §66).
    Imxrt,
    /// nRF52840 SAADC with EasyDMA (nRF52840 PS §6.23).
    Saadc { ram: u64 },
    /// SAM4S 12-bit ADC: `CHER` channel set, `CR.START`, `CDRx`/`LCDR`
    /// (SAM4S datasheet DS11100 §42).
    Sam4s,
    /// Ambiq Apollo4 ADC: SLOTnCFG slots, software trigger `SWT` = 0x37,
    /// slot-tagged results in the FIFO with six fractional bits, against the
    /// 1.19 V internal reference, so the 3.3 V netlists saturate above it.
    Apollo4,
    /// CC2538 SoC ADC: an `ADCCON3` write runs one extra conversion, EOC in
    /// `ADCCON1` clears on the `ADCH` read, and the 2's-complement result
    /// sits MSB-aligned in `ADCH:ADCL` (SWRU319C §15, Contiki-NG adc.c).
    Cc2538,
}

struct Converter {
    id: String,
    base: u64,
    family: Family,
}

/// Every non-stub ADC the descriptor declares, with the family it converts
/// through. `Err` names an ADC this test cannot convert and why.
fn converters(chip: &ChipDescriptor) -> Result<Vec<Converter>, String> {
    let mut found = Vec::new();
    for p in &chip.peripherals {
        let kind = p.r#type.to_ascii_lowercase();
        if kind == "stub" || !p.id.to_ascii_lowercase().contains("adc") {
            continue;
        }
        let family = if kind == "imxrt_adc" {
            Family::Imxrt
        } else if kind.contains("saadc") {
            Family::Saadc { ram: chip.ram.base }
        } else if kind == "sam4s_adc" {
            Family::Sam4s
        } else if kind == "apollo4_adc" {
            Family::Apollo4
        } else if kind == "cc2538_soc_adc" {
            Family::Cc2538
        } else {
            match p.config.get("profile").and_then(|v| v.as_str()) {
                Some("stm32f4") => Family::Stm32F4,
                Some("stm32h7") => Family::Stm32H7,
                Some("stm32l4") => Family::Stm32L4,
                Some("stm32f0" | "stm32l0") => Family::Stm32V2,
                Some("stm32wba") => Family::Stm32Adc4,
                other => {
                    return Err(format!(
                        "{}:{} ({kind}, profile {other:?})",
                        chip.name, p.id
                    ))
                }
            }
        };
        found.push(Converter {
            id: p.id.clone(),
            base: p.base_address,
            family,
        });
    }
    Ok(found)
}

impl Converter {
    fn expected_code(&self, volts: f64) -> u32 {
        let mv = millivolts(volts);
        match self.family {
            // 12-bit, mV * 4095 / VREF (3.3 V) truncated.
            Family::Stm32F4
            | Family::Stm32H7
            | Family::Stm32L4
            | Family::Stm32V2
            | Family::Stm32Adc4
            | Family::Sam4s => (mv * 4095 / 3300).min(4095),
            // 12-bit, rounded to nearest.
            Family::Imxrt => (mv.min(3300) * 4095 + 1650) / 3300,
            // Gain 1/4 against VDD/4: full scale is VDD, 12-bit, 2^12 / VDD.
            Family::Saadc { .. } => (mv * 4096 / 3300).min(4095),
            // 12-bit against VREF 1.19 V, truncated, clamped to full scale.
            Family::Apollo4 => (mv * 4095 / 1190).min(4095),
            // Decimation 512 against AVDD5 (3.3 V): 12 bits including the
            // sign, so single-ended full scale is 2047.
            Family::Cc2538 => (mv * 2047 / 3300).min(2047),
        }
    }

    /// Bring the converter up the way firmware does, before any conversion.
    fn power_up(&self, bus: &mut SystemBus) {
        match self.family {
            Family::Stm32F4 | Family::Sam4s => {}
            Family::Cc2538 => {
                // adc_init(): ADCCON1.STSEL = 11, manual starts only.
                bus.write_u32(self.base, 0x30).expect("ADCCON1.STSEL");
            }
            Family::Apollo4 => {
                // CFG.ADCEN with destructive FIFO reads (DFIFORDEN).
                bus.write_u32(self.base, 1 | (1 << 12))
                    .expect("enable Apollo4 ADC");
            }
            Family::Stm32H7 => {
                let cr = self.base + 0x08;
                bus.write_u32(cr, 0).expect("leave H7 ADC deep power-down");
                bus.write_u32(cr, 1 << 28).expect("enable ADC regulator");
                bus.write_u32(cr, (1 << 28) | (1 << 31))
                    .expect("calibrate ADC");
                bus.write_u32(cr, (1 << 28) | 1).expect("enable ADC");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: H7 ADC reports ADRDY",
                    self.id
                );
                bus.write_u32(self.base + 0x0C, 2 << 2)
                    .expect("select 12-bit H7 resolution");
                bus.write_u32(self.base + 0x1C, (1 << NTC_CHANNEL) | (1 << DIODE_CHANNEL))
                    .expect("preselect routed channels");
            }
            Family::Stm32L4 => {
                let cr = self.base + 0x08;
                assert_eq!(
                    bus.read_u32(cr).unwrap(),
                    1 << 29,
                    "{}: L4 CR resets in deep power-down",
                    self.id
                );
                bus.write_u32(cr, 0).expect("leave L4 ADC deep power-down");
                bus.write_u32(cr, 1 << 28).expect("enable ADC regulator");
                bus.write_u32(cr, (1 << 28) | 1).expect("enable ADC");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: L4 ADC reports ADRDY",
                    self.id
                );
                // CFGR RES[4:3] = 00 selects 12 bits, right-aligned.
                bus.write_u32(self.base + 0x0C, 0).expect("select 12-bit");
            }
            Family::Stm32V2 => {
                const ADEN: u32 = 1;
                const ADCAL: u32 = 1 << 31;
                const EOCAL: u32 = 1 << 11;
                let cr = self.base + 0x08;
                assert_eq!(
                    bus.read_u32(cr).unwrap(),
                    0,
                    "{}: v2 CR resets to 0, no DEEPPWD",
                    self.id
                );
                bus.write_u32(cr, ADCAL).expect("calibrate ADC");
                assert_eq!(bus.read_u32(cr).unwrap() & ADCAL, 0, "ADCAL self-clears");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & EOCAL,
                    0,
                    "{}: calibration raises EOCAL",
                    self.id
                );
                bus.write_u32(self.base, EOCAL).expect("clear EOCAL");
                bus.write_u32(cr, ADEN).expect("enable ADC");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: ADEN raises ADRDY",
                    self.id
                );
                // CFGR1 RES[4:3] = 00 selects 12 bits, right-aligned.
                bus.write_u32(self.base + 0x0C, 0).expect("select 12-bit");
            }
            Family::Stm32Adc4 => {
                const ADEN: u32 = 1;
                const ADVREGEN: u32 = 1 << 28;
                const ADCAL: u32 = 1 << 31;
                const LDORDY: u32 = 1 << 12;
                const EOCAL: u32 = 1 << 11;
                let cr = self.base + 0x08;
                assert_eq!(
                    bus.read_u32(cr).unwrap(),
                    0,
                    "{}: ADC4 CR resets to 0",
                    self.id
                );
                bus.write_u32(cr, ADEN).expect("ADEN before the LDO");
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: ADC4 stays not-ready until ADVREGEN",
                    self.id
                );
                bus.write_u32(cr, ADVREGEN).expect("enable ADC4 LDO");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & LDORDY,
                    0,
                    "{}: ADVREGEN raises LDORDY",
                    self.id
                );
                bus.write_u32(cr, ADVREGEN | ADCAL).expect("calibrate ADC4");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & EOCAL,
                    0,
                    "{}: calibration raises EOCAL",
                    self.id
                );
                bus.write_u32(self.base, EOCAL | LDORDY)
                    .expect("clear EOCAL/LDORDY");
                bus.write_u32(cr, ADVREGEN | ADEN).expect("enable ADC4");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: ADEN raises ADRDY",
                    self.id
                );
                // CFGR1 RES[3:2] = 00 selects 12 bits, right-aligned.
                bus.write_u32(self.base + 0x0C, 0).expect("select 12-bit");
            }
            Family::Imxrt => {
                let cfg = bus.read_u32(self.base + 0x44).unwrap();
                bus.write_u32(self.base + 0x44, (cfg & !(3 << 2)) | (2 << 2))
                    .expect("select 12-bit i.MX RT ADC mode");
            }
            Family::Saadc { ram } => {
                for (slot, channel) in [(0u64, NTC_CHANNEL), (1, DIODE_CHANNEL), (2, IDLE_CHANNEL)]
                {
                    let ch = self.base + 0x510 + slot * 0x10;
                    bus.write_u32(ch, u32::from(channel) + 1)
                        .expect("select SAADC PSELP = AIN<channel>");
                    bus.write_u32(ch + 8, 0x1200)
                        .expect("select gain 1/4 and VDD/4 reference");
                }
                bus.write_u32(self.base + 0x5f0, 2)
                    .expect("select 12-bit SAADC resolution");
                bus.write_u32(self.base + 0x62c, ram as u32)
                    .expect("set EasyDMA result pointer");
                bus.write_u32(self.base + 0x630, 3)
                    .expect("reserve three results");
                bus.write_u32(self.base + 0x500, 1).expect("enable SAADC");
            }
        }
    }

    /// One firmware-style conversion per routed channel: `[ntc, diode]`.
    fn convert_routed(&self, bus: &mut SystemBus) -> [u32; 2] {
        match self.family {
            Family::Saadc { ram } => {
                bus.write_u32(self.base + 0x104, 0).expect("clear END");
                bus.write_u32(self.base, 1).expect("TASKS_START");
                bus.write_u32(self.base + 4, 1).expect("TASKS_SAMPLE");
                bus.tick_peripherals_fully();
                assert_eq!(bus.read_u32(self.base + 0x104).unwrap(), 1, "scan sets END");
                assert_eq!(bus.read_u32(self.base + 0x634).unwrap(), 3, "AMOUNT");
                assert_eq!(
                    bus.read_u16(ram + 4).unwrap(),
                    0,
                    "{}: unrouted AIN{IDLE_CHANNEL} converts to zero",
                    self.id
                );
                [
                    u32::from(bus.read_u16(ram).unwrap()),
                    u32::from(bus.read_u16(ram + 2).unwrap()),
                ]
            }
            Family::Sam4s => {
                // One START converts the whole enabled set in channel
                // order; EMR.TAG stamps LCDR with the last channel.
                const EMR_TAG: u32 = 1 << 24;
                const DRDY: u32 = 1 << 24;
                let set = (1 << NTC_CHANNEL) | (1 << DIODE_CHANNEL) | (1 << IDLE_CHANNEL);
                bus.write_u32(self.base + 0x40, EMR_TAG).expect("EMR.TAG");
                bus.write_u32(self.base + 0x10, set).expect("CHER");
                bus.write_u32(self.base, 1 << 1).expect("CR.START");
                assert_eq!(
                    bus.read_u32(self.base + 0x30).unwrap() & (set | DRDY),
                    set | DRDY,
                    "{}: START raises EOC for every enabled channel and DRDY",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x20).unwrap(),
                    u32::from(IDLE_CHANNEL) << 12,
                    "{}: LCDR tags the last channel, unrouted AD{IDLE_CHANNEL}, with code 0",
                    self.id
                );
                let codes = [
                    bus.read_u32(self.base + 0x50 + 4 * u64::from(NTC_CHANNEL))
                        .unwrap(),
                    bus.read_u32(self.base + 0x50 + 4 * u64::from(DIODE_CHANNEL))
                        .unwrap(),
                ];
                assert_eq!(
                    bus.read_u32(self.base + 0x30).unwrap() & (set | DRDY),
                    1 << IDLE_CHANNEL,
                    "{}: CDR reads clear their EOC, the LCDR read cleared DRDY",
                    self.id
                );
                bus.write_u32(self.base + 0x14, set).expect("CHDR");
                bus.read_u32(self.base + 0x50 + 4 * u64::from(IDLE_CHANNEL))
                    .expect("drain the idle CDR");
                codes
            }
            Family::Cc2538 => {
                // One extra conversion per channel; EOC rises at the ADCCON3
                // write and the ADCH read clears it before the next.
                [NTC_CHANNEL, DIODE_CHANNEL].map(|channel| {
                    let code = self.convert(bus, channel);
                    assert_eq!(
                        bus.read_u32(self.base).unwrap(),
                        0x30,
                        "{}: EOC clear after the ADCH read, STSEL kept",
                        self.id
                    );
                    code
                })
            }
            Family::Apollo4 => {
                // Three slots, one SWT: each slot pushes one FIFO entry
                // tagged with its slot number, in slot order.
                for (slot, channel) in [(0u64, NTC_CHANNEL), (1, DIODE_CHANNEL), (2, IDLE_CHANNEL)]
                {
                    bus.write_u32(self.base + 0x0C + 4 * slot, 1 | (u32::from(channel) << 8))
                        .expect("enable 12-bit slot");
                }
                bus.write_u32(self.base + 0x08, 0x37).expect("SWT");
                assert_eq!(
                    bus.read_u32(self.base + 0x204).unwrap() & 3,
                    3,
                    "{}: the scan raises CNVCMP and SCNCMP",
                    self.id
                );
                bus.write_u32(self.base + 0x208, 3).expect("INTCLR");
                let mut codes = [0; 3];
                for (slot, code) in codes.iter_mut().enumerate() {
                    let word = bus.read_u32(self.base + 0x3C).expect("pop FIFOPR");
                    assert_eq!(
                        (word >> 28) & 7,
                        slot as u32,
                        "{}: FIFO entry {slot} carries its slot number",
                        self.id
                    );
                    assert_eq!(
                        (word >> 20) & 0xFF,
                        3 - slot as u32,
                        "{}: FIFO count before pop {slot}",
                        self.id
                    );
                    *code = (word & 0xF_FFFF) >> 6;
                }
                assert_eq!(
                    codes[2], 0,
                    "{}: unrouted SE{IDLE_CHANNEL} converts to zero",
                    self.id
                );
                for slot in 0..3u64 {
                    bus.write_u32(self.base + 0x0C + 4 * slot, 0)
                        .expect("disable slot");
                }
                [codes[0], codes[1]]
            }
            Family::Stm32V2 | Family::Stm32Adc4 => {
                let single = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                let idle = self.convert(bus, IDLE_CHANNEL);
                let [ntc, diode] = single;
                for (cfgr1, chselr, order) in self.v2_scans() {
                    let expected: Vec<u32> = order
                        .iter()
                        .map(|&ch| match ch {
                            NTC_CHANNEL => ntc,
                            DIODE_CHANNEL => diode,
                            _ => idle,
                        })
                        .collect();
                    assert_eq!(
                        self.scan_v2(bus, cfgr1, chselr, order.len()),
                        expected,
                        "{}: one ADSTART scans channels {order:?} into DR in turn",
                        self.id
                    );
                }
                single
            }
            Family::Stm32F4 => {
                let single = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                let [ntc, diode] = single;
                for (order, expected) in [
                    ([NTC_CHANNEL, DIODE_CHANNEL], [ntc, diode]),
                    ([DIODE_CHANNEL, NTC_CHANNEL], [diode, ntc]),
                ] {
                    assert_eq!(
                        self.scan_f4(bus, &order),
                        expected,
                        "{}: one SWSTART scans CR1.SCAN group {order:?} into DR in turn",
                        self.id
                    );
                }
                // CR1.DISCEN over [NTC, diode, NTC]: one sensor per trigger,
                // then two, each subgroup stopping short at SQ3.
                let group = [NTC_CHANNEL, DIODE_CHANNEL, NTC_CHANNEL];
                assert_eq!(
                    self.discen_f4(bus, &group, 1, 4),
                    vec![vec![ntc], vec![diode], vec![ntc], vec![ntc]],
                    "{}: DISCNUM=0 converts one routed sensor per SWSTART",
                    self.id
                );
                assert_eq!(
                    self.discen_f4(bus, &group, 2, 3),
                    vec![vec![ntc, diode], vec![ntc], vec![ntc, diode]],
                    "{}: DISCNUM=1 converts the next two routed sensors per SWSTART",
                    self.id
                );
                // Injected group: one JSWSTART converts both routed sensors
                // into JDR1..JDRn in rank order.
                for (ranks, expected) in [
                    (vec![DIODE_CHANNEL], vec![diode]),
                    (
                        vec![NTC_CHANNEL, DIODE_CHANNEL, NTC_CHANNEL],
                        vec![ntc, diode, ntc],
                    ),
                ] {
                    assert_eq!(
                        self.injected_f4(bus, &ranks),
                        expected,
                        "{}: one JSWSTART converts injected ranks {ranks:?}",
                        self.id
                    );
                }
                single
            }
            Family::Stm32H7 | Family::Stm32L4 => {
                let single = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                let idle = self.convert(bus, IDLE_CHANNEL);
                let [ntc, diode] = single;
                for (sqr, order) in Self::sqr_scans() {
                    let expected: Vec<u32> = order
                        .iter()
                        .map(|&ch| match ch {
                            NTC_CHANNEL => ntc,
                            DIODE_CHANNEL => diode,
                            _ => idle,
                        })
                        .collect();
                    assert_eq!(
                        self.scan_sqr(bus, sqr, order.len()),
                        expected,
                        "{}: one ADSTART scans SQR sequence {order:?} into DR in turn",
                        self.id
                    );
                }
                // DISCEN, DISCNUM = 1: NTC, diode, idle, NTC, diode in
                // subsequences of two; the third stops at the sequence end.
                assert_eq!(
                    self.scan_sqr_discontinuous(bus, 1, 5),
                    vec![
                        vec![ntc, diode],
                        vec![idle, ntc],
                        vec![diode],
                        vec![ntc, diode]
                    ],
                    "{}: each ADSTART converts the next DISCNUM + 1 channels",
                    self.id
                );
                // Injected group: one JADSTART converts the routed sensors
                // into JDR1..JDRn in JSQ order, leaving DR alone.
                let dr = bus.read_u32(self.base + 0x40).expect("DR");
                for (ranks, expected) in [
                    (vec![DIODE_CHANNEL], vec![diode]),
                    (
                        vec![NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL, NTC_CHANNEL],
                        vec![ntc, diode, idle, ntc],
                    ),
                ] {
                    assert_eq!(
                        self.injected_sqr(bus, &ranks),
                        expected,
                        "{}: one JADSTART converts injected ranks {ranks:?}",
                        self.id
                    );
                }
                assert_eq!(
                    bus.read_u32(self.base + 0x40).unwrap(),
                    dr,
                    "{}: the injected group leaves DR alone",
                    self.id
                );
                single
            }
            _ => [
                self.convert(bus, NTC_CHANNEL),
                self.convert(bus, DIODE_CHANNEL),
            ],
        }
    }

    /// `(CFGR1, CHSELR, channel order)` for the multi-channel scans each v2
    /// map runs. The bitmap scans upward, or downward with SCANDIR; ADC4's
    /// CHSELRMOD sequence runs in SQx order whatever the channel numbers.
    fn v2_scans(&self) -> Vec<(u32, u32, Vec<u8>)> {
        let bitmap = (1 << NTC_CHANNEL) | (1 << DIODE_CHANNEL) | (1 << IDLE_CHANNEL);
        let upward = vec![NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL];
        let downward = vec![IDLE_CHANNEL, DIODE_CHANNEL, NTC_CHANNEL];
        match self.family {
            Family::Stm32V2 => vec![(0, bitmap, upward), (1 << 2, bitmap, downward)],
            Family::Stm32Adc4 => {
                // SQ1 = diode, SQ2 = NTC, SQ3 = 0xF ends the sequence.
                let sq = 0xFFFF_FF00 | (u32::from(NTC_CHANNEL) << 4) | u32::from(DIODE_CHANNEL);
                vec![
                    (0, bitmap, upward),
                    (1 << 4, bitmap, downward),
                    (1 << 21, sq, vec![DIODE_CHANNEL, NTC_CHANNEL]),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// `([SQR1..SQR4], channel order)` for the multi-channel scans the L4 and
    /// H7 maps run. `SQR1.L` is the length minus one; SQ1..SQ4 follow it in
    /// SQR1, SQ5.. continue five to a word in SQR2. The order is the SQx order
    /// whatever the channel numbers, so the reversed pair converts the diode
    /// first, and the six-long one reaches into SQR2.
    fn sqr_scans() -> Vec<([u32; 4], Vec<u8>)> {
        let [ntc, diode, idle] = [NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL].map(u32::from);
        vec![
            (
                [2 | (ntc << 6) | (diode << 12) | (idle << 18), 0, 0, 0],
                vec![NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL],
            ),
            (
                [1 | (diode << 6) | (ntc << 12), 0, 0, 0],
                vec![DIODE_CHANNEL, NTC_CHANNEL],
            ),
            (
                [
                    5 | (idle << 6) | (idle << 12) | (idle << 18) | (idle << 24),
                    ntc | (diode << 6),
                    0,
                    0,
                ],
                vec![
                    IDLE_CHANNEL,
                    IDLE_CHANNEL,
                    IDLE_CHANNEL,
                    IDLE_CHANNEL,
                    NTC_CHANNEL,
                    DIODE_CHANNEL,
                ],
            ),
        ]
    }

    /// One software-started L4/H7 `SQRx` scan read by polling. Leaves SQR1..4
    /// at a one-channel sequence.
    fn scan_sqr(&self, bus: &mut SystemBus, sqr: [u32; 4], len: usize) -> Vec<u32> {
        const EOC: u32 = 1 << 2;
        const EOS: u32 = 1 << 3;
        for (k, word) in sqr.into_iter().enumerate() {
            bus.write_u32(self.base + 0x30 + 4 * k as u64, word)
                .expect("write SQRx");
        }
        bus.write_u32(self.base, EOC | EOS).expect("clear EOC/EOS");
        bus.write_u32(self.base + 0x08, (1 << 28) | 1 | (1 << 2))
            .expect("ADSTART");
        let codes = self.poll_scan(bus, len);
        for k in 0..4u64 {
            bus.write_u32(self.base + 0x30 + 4 * k, 0)
                .expect("restore SQRx");
        }
        codes
    }

    /// Four software triggers of a discontinuous (`CFGR.DISCEN`) L4/H7
    /// `SQRx` sequence of `len` channels cycling NTC, diode, idle, read by
    /// polling. Each converts the next `discnum` + 1 channels; only the one
    /// reaching the sequence end raises `EOS`. Restores CFGR and SQR1..4.
    fn scan_sqr_discontinuous(
        &self,
        bus: &mut SystemBus,
        discnum: u32,
        len: usize,
    ) -> Vec<Vec<u32>> {
        const EOC: u32 = 1 << 2;
        const EOS: u32 = 1 << 3;
        let cycle = [NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL].map(u32::from);
        let mut sqr = [len as u32 - 1, 0, 0, 0];
        for slot in 0..len {
            let (word, field) = match slot {
                0..=3 => (0, slot + 1),
                _ => (1, slot - 4),
            };
            sqr[word] |= cycle[slot % 3] << (6 * field);
        }
        for (k, word) in sqr.into_iter().enumerate() {
            bus.write_u32(self.base + 0x30 + 4 * k as u64, word)
                .expect("write SQRx");
        }
        let cfgr = bus.read_u32(self.base + 0x0C).expect("CFGR");
        bus.write_u32(self.base + 0x0C, cfgr | (1 << 16) | (discnum << 17))
            .expect("DISCEN");
        let per_trigger = discnum as usize + 1;
        let mut position = 0;
        let mut triggers = Vec::new();
        for _ in 0..4 {
            let n = per_trigger.min(len - position);
            bus.write_u32(self.base, EOC | EOS).expect("clear EOC/EOS");
            bus.write_u32(self.base + 0x08, (1 << 28) | 1 | (1 << 2))
                .expect("ADSTART");
            let codes: Vec<u32> = (0..n)
                .map(|_| {
                    let isr = bus.read_u32(self.base).unwrap();
                    assert_ne!(isr & EOC, 0, "{}: EOC", self.id);
                    assert_eq!(
                        isr & EOS != 0,
                        position + n == len,
                        "{}: EOS only on the subsequence reaching SQ{len}",
                        self.id
                    );
                    bus.write_u32(self.base, EOC).expect("clear EOC");
                    bus.read_u32(self.base + 0x40).expect("read DR")
                })
                .collect();
            assert_eq!(
                bus.read_u32(self.base).unwrap() & EOC,
                0,
                "{}: the subsequence stops after {n}",
                self.id
            );
            triggers.push(codes);
            position = (position + n) % len;
        }
        bus.write_u32(self.base + 0x0C, cfgr).expect("restore CFGR");
        for k in 0..4u64 {
            bus.write_u32(self.base + 0x30 + 4 * k, 0)
                .expect("restore SQRx");
        }
        triggers
    }

    /// One software-started F2/F4/F7 `CR1.SCAN` group with `CR2.EOCS`, read
    /// by polling: wait for EOC, clear it, read DR. Leaves CR1/SQR1 at 0.
    fn scan_f4(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        const ADON: u32 = 1;
        const EOCS: u32 = 1 << 10;
        let sqr3 = channels
            .iter()
            .enumerate()
            .fold(0, |w, (k, &ch)| w | (u32::from(ch) << (5 * k)));
        bus.write_u32(self.base + 0x34, sqr3).expect("SQR3 SQ1..");
        bus.write_u32(self.base + 0x2c, (channels.len() as u32 - 1) << 20)
            .expect("SQR1.L");
        bus.write_u32(self.base + 0x04, 1 << 8).expect("CR1.SCAN");
        bus.write_u32(self.base, 0).expect("clear SR");
        bus.write_u32(self.base + 0x08, ADON | EOCS).expect("ADON");
        bus.write_u32(self.base + 0x08, ADON | EOCS | (1 << 30))
            .expect("SWSTART");
        let codes = (0..channels.len())
            .map(|slot| {
                let mut ticks = 0;
                while bus.read_u32(self.base).unwrap() & (1 << 1) == 0 {
                    assert!(ticks < 64, "{}: EOC for conversion {slot}", self.id);
                    bus.tick_peripherals_fully();
                    ticks += 1;
                }
                bus.write_u32(self.base, 0).expect("clear EOC");
                bus.read_u32(self.base + 0x4c).expect("read DR")
            })
            .collect();
        bus.write_u32(self.base + 0x04, 0).expect("restore CR1");
        bus.write_u32(self.base + 0x2c, 0).expect("restore SQR1");
        codes
    }

    /// F2/F4/F7 injected group over `channels` (rank 1 first), placed the
    /// way the ST HAL's `ADC_JSQR_RK_JL` does — rank r of n in `JSQ(4 - n +
    /// r)` — and started by `CR2.JSWSTART` (bit 22, RM0090 §13.13.3). Waits
    /// for `JEOC` and returns `JDR1..JDRn`.
    fn injected_f4(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        const ADON: u32 = 1;
        let n = channels.len();
        let jsqr = channels
            .iter()
            .enumerate()
            .fold(((n as u32) - 1) << 20, |w, (r, &ch)| {
                w | (u32::from(ch) << (5 * (4 - n + r)))
            });
        bus.write_u32(self.base + 0x38, jsqr).expect("JSQR");
        bus.write_u32(self.base, 0).expect("clear SR");
        bus.write_u32(self.base + 0x08, ADON).expect("ADON");
        bus.write_u32(self.base + 0x08, ADON | (1 << 22))
            .expect("JSWSTART");
        let mut ticks = 0;
        while bus.read_u32(self.base).unwrap() & (1 << 2) == 0 {
            assert!(ticks < 64 * n, "{}: JEOC", self.id);
            bus.tick_peripherals_fully();
            ticks += 1;
        }
        (0..n)
            .map(|k| bus.read_u32(self.base + 0x3c + 4 * k as u64).expect("JDRx"))
            .collect()
    }

    /// L4/L5/H7 injected group over `channels` (`JSQ1` first), started by
    /// `CR.JADSTART` (bit 3) with a software trigger. The L4 map (L4, L5)
    /// packs `JSQR` with `JSQ1` at bit 8 (`stm32l476.svd`); the H7 with
    /// `JSQ1` at bit 9 (`stm32h735.svd`). Expects `JEOC`|`JEOS` and
    /// `JADSTART` cleared, and returns `JDR1..JDRn`.
    fn injected_sqr(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        const JEOC_JEOS: u32 = (1 << 5) | (1 << 6);
        const JADSTART: u32 = 1 << 3;
        let jsq1 = if matches!(self.family, Family::Stm32H7) {
            9
        } else {
            8
        };
        let n = channels.len();
        let jsqr = channels
            .iter()
            .enumerate()
            .fold((n as u32) - 1, |w, (k, &ch)| {
                w | (u32::from(ch) << (jsq1 + 6 * k))
            });
        bus.write_u32(self.base + 0x4C, jsqr).expect("JSQR");
        bus.write_u32(self.base, JEOC_JEOS)
            .expect("clear JEOC/JEOS");
        bus.write_u32(self.base + 0x08, (1 << 28) | 1 | JADSTART)
            .expect("JADSTART");
        assert_eq!(
            bus.read_u32(self.base).unwrap() & JEOC_JEOS,
            JEOC_JEOS,
            "{}: the injected group sets JEOC and JEOS",
            self.id
        );
        assert_eq!(
            bus.read_u32(self.base + 0x08).unwrap() & JADSTART,
            0,
            "{}: JADSTART self-clears",
            self.id
        );
        (0..n)
            .map(|k| bus.read_u32(self.base + 0x80 + 4 * k as u64).expect("JDRx"))
            .collect()
    }

    /// F2/F4/F7 `CR1.DISCEN` with `DISCNUM` = `per_trigger` - 1 and
    /// `CR2.EOCS`, read by polling: each SWSTART lands the next
    /// `per_trigger` channels (fewer at the group end), one `EOC` each, and
    /// then the ADC stays idle. Leaves CR1/SQR1 at 0.
    fn discen_f4(
        &self,
        bus: &mut SystemBus,
        channels: &[u8],
        per_trigger: usize,
        triggers: usize,
    ) -> Vec<Vec<u32>> {
        const ADON: u32 = 1;
        const EOCS: u32 = 1 << 10;
        let sqr3 = channels
            .iter()
            .enumerate()
            .fold(0, |w, (k, &ch)| w | (u32::from(ch) << (5 * k)));
        bus.write_u32(self.base + 0x34, sqr3).expect("SQR3 SQ1..");
        bus.write_u32(self.base + 0x2c, (channels.len() as u32 - 1) << 20)
            .expect("SQR1.L");
        let cr1 = (1 << 8) | (1 << 11) | ((per_trigger as u32 - 1) << 13);
        bus.write_u32(self.base + 0x04, cr1).expect("CR1.DISCEN");
        bus.write_u32(self.base, 0).expect("clear SR");
        bus.write_u32(self.base + 0x08, ADON | EOCS).expect("ADON");
        let mut next = 0;
        let out = (0..triggers)
            .map(|t| {
                bus.write_u32(self.base + 0x08, ADON | EOCS | (1 << 30))
                    .expect("SWSTART");
                let n = per_trigger.min(channels.len() - next);
                next = (next + n) % channels.len();
                let codes = (0..n)
                    .map(|slot| {
                        let mut ticks = 0;
                        while bus.read_u32(self.base).unwrap() & (1 << 1) == 0 {
                            assert!(ticks < 64, "{}: trigger {t} EOC {slot}", self.id);
                            bus.tick_peripherals_fully();
                            ticks += 1;
                        }
                        bus.write_u32(self.base, 0).expect("clear EOC");
                        bus.read_u32(self.base + 0x4c).expect("read DR")
                    })
                    .collect();
                for _ in 0..64 {
                    bus.tick_peripherals_fully();
                }
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & (1 << 1),
                    0,
                    "{}: trigger {t} stops after its {n}-channel subgroup",
                    self.id
                );
                codes
            })
            .collect();
        bus.write_u32(self.base + 0x04, 0).expect("restore CR1");
        bus.write_u32(self.base + 0x2c, 0).expect("restore SQR1");
        out
    }

    /// One software-started v2 scan read by polling: wait for EOC, read DR.
    /// EOS must stay low until the last conversion lands. Leaves CFGR1 at 0.
    fn scan_v2(&self, bus: &mut SystemBus, cfgr1: u32, chselr: u32, len: usize) -> Vec<u32> {
        const EOC: u32 = 1 << 2;
        const EOS: u32 = 1 << 3;
        let ldo = if matches!(self.family, Family::Stm32Adc4) {
            1 << 28
        } else {
            0
        };
        bus.write_u32(self.base + 0x0C, cfgr1).expect("write CFGR1");
        bus.write_u32(self.base + 0x28, chselr)
            .expect("write CHSELR");
        bus.write_u32(self.base, EOC | EOS).expect("clear EOC/EOS");
        bus.write_u32(self.base + 0x08, ldo | 1 | (1 << 2))
            .expect("ADSTART");
        let codes = self.poll_scan(bus, len);
        bus.write_u32(self.base + 0x0C, 0).expect("restore CFGR1");
        codes
    }

    /// Read a running `len`-conversion scan the way polling firmware does:
    /// wait for EOC, clear it, read DR. EOS must stay low until the last
    /// conversion lands.
    fn poll_scan(&self, bus: &mut SystemBus, len: usize) -> Vec<u32> {
        const EOC: u32 = 1 << 2;
        const EOS: u32 = 1 << 3;
        (0..len)
            .map(|slot| {
                let isr = bus.read_u32(self.base).unwrap();
                assert_ne!(isr & EOC, 0, "{}: EOC for conversion {slot}", self.id);
                assert_eq!(
                    isr & EOS != 0,
                    slot == len - 1,
                    "{}: EOS only when the last of {len} conversions lands (at {slot})",
                    self.id
                );
                bus.write_u32(self.base, EOC).expect("clear EOC");
                bus.read_u32(self.base + 0x40).expect("read DR")
            })
            .collect()
    }

    fn convert(&self, bus: &mut SystemBus, channel: u8) -> u32 {
        match self.family {
            Family::Stm32F4 => {
                const ADON: u32 = 1;
                bus.write_u32(self.base + 0x34, u32::from(channel))
                    .expect("select SQR3 SQ1");
                bus.write_u32(self.base + 0x08, ADON).expect("ADON");
                bus.write_u32(self.base + 0x08, ADON | (1 << 30))
                    .expect("SWSTART");
                for _ in 0..64 {
                    bus.tick_peripherals_fully();
                }
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & (1 << 1),
                    0,
                    "{}: conversion sets EOC",
                    self.id
                );
                bus.read_u32(self.base + 0x4c).expect("read DR")
            }
            Family::Stm32H7 | Family::Stm32L4 => {
                const EOC_EOS: u32 = (1 << 2) | (1 << 3);
                bus.write_u32(self.base + 0x30, u32::from(channel) << 6)
                    .expect("select SQR1 SQ1");
                bus.write_u32(self.base + 0x08, (1 << 28) | 1 | (1 << 2))
                    .expect("ADSTART");
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & EOC_EOS,
                    EOC_EOS,
                    "{}: SQ1 conversion sets EOC and EOS",
                    self.id
                );
                let code = bus.read_u32(self.base + 0x40).expect("read DR");
                bus.write_u32(self.base, EOC_EOS).expect("clear EOC/EOS");
                code
            }
            Family::Stm32V2 | Family::Stm32Adc4 => {
                const EOC_EOS: u32 = (1 << 2) | (1 << 3);
                // ADC4 keeps ADVREGEN on while it converts.
                let ldo = if matches!(self.family, Family::Stm32Adc4) {
                    1 << 28
                } else {
                    0
                };
                bus.write_u32(self.base + 0x28, 1 << channel)
                    .expect("select CHSELR channel");
                bus.write_u32(self.base, EOC_EOS).expect("clear EOC/EOS");
                bus.write_u32(self.base + 0x08, ldo | 1 | (1 << 2))
                    .expect("ADSTART");
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & EOC_EOS,
                    EOC_EOS,
                    "{}: single-channel CHSELR conversion sets EOC and EOS",
                    self.id
                );
                bus.read_u32(self.base + 0x40).expect("read DR")
            }
            Family::Imxrt => {
                bus.write_u32(self.base, u32::from(channel))
                    .expect("HC0 software trigger");
                assert_eq!(
                    bus.read_u32(self.base + 0x20).unwrap() & 1,
                    0,
                    "{}: HS.COCO0 clears when HC0 starts a conversion",
                    self.id
                );
                // A conversion takes 3 us; publish 10 us at the RT1064's
                // 600 MHz core (the model's default without a Machine) on
                // the bus clock the model reads.
                let now = bus.current_cycle;
                bus.set_current_cycle(now + 6_000);
                assert_ne!(
                    bus.read_u32(self.base + 0x20).unwrap() & 1,
                    0,
                    "{}: HS.COCO0 sets once the conversion time has elapsed",
                    self.id
                );
                bus.read_u32(self.base + 0x24).expect("read R0")
            }
            Family::Sam4s => {
                bus.write_u32(self.base + 0x10, 1 << channel).expect("CHER");
                bus.write_u32(self.base, 1 << 1).expect("CR.START");
                let code = bus
                    .read_u32(self.base + 0x50 + 4 * u64::from(channel))
                    .expect("read CDRx");
                bus.write_u32(self.base + 0x14, 1 << channel).expect("CHDR");
                code
            }
            Family::Apollo4 => {
                bus.write_u32(self.base + 0x0C, 1 | (u32::from(channel) << 8))
                    .expect("enable slot 0");
                bus.write_u32(self.base + 0x08, 0x37).expect("SWT");
                let word = bus.read_u32(self.base + 0x3C).expect("pop FIFOPR");
                bus.write_u32(self.base + 0x0C, 0).expect("disable slot 0");
                (word & 0xF_FFFF) >> 6
            }
            Family::Cc2538 => {
                // Contiki-NG adc_get(): EREF = AVDD5, EDIV = 512, ECH = AINn.
                bus.write_u32(self.base + 0x08, (2 << 6) | (3 << 4) | u32::from(channel))
                    .expect("ADCCON3 extra conversion");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 0x80,
                    0,
                    "{}: ADCCON1.EOC",
                    self.id
                );
                let low = bus.read_u32(self.base + 0x0C).expect("ADCL") & 0xFC;
                let high = bus.read_u32(self.base + 0x10).expect("ADCH");
                let raw = (low | (high << 8)) as u16 as i16;
                assert!(raw >= 0, "{}: single-ended results are positive", self.id);
                assert_eq!(raw & 0xF, 0, "{}: 12 significant bits", self.id);
                (raw >> 4) as u32
            }
            Family::Saadc { .. } => unreachable!("SAADC converts as a scan"),
        }
    }

    /// The shared STM32 model also exposes what it holds per channel, so an
    /// unrouted channel can be shown undriven rather than merely "some code".
    fn held_count(&self, bus: &mut SystemBus, channel: u8) -> Option<u16> {
        if !matches!(
            self.family,
            Family::Stm32F4
                | Family::Stm32H7
                | Family::Stm32L4
                | Family::Stm32V2
                | Family::Stm32Adc4
        ) {
            return None;
        }
        let index = bus.find_peripheral_index_by_name(&self.id)?;
        Some(
            bus.peripherals[index]
                .dev
                .as_any_mut()?
                .downcast_mut::<labwired_core::peripherals::adc::Adc>()
                .expect("STM32 ADC uses the shared model")
                .channel_input_count(channel),
        )
    }
}

fn generic_netlists(adc_id: &str) -> SystemManifest {
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, channel) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", NTC_CHANNEL),
            "diode_load_line" => ("v_diode", DIODE_CHANNEL),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("adc.{adc_id}.{channel}_volts"));
    }
    analog
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

fn assert_converter_routes_generic_analog(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
    converter: &Converter,
) {
    let tag = format!("{board}:{}", converter.id);
    let mut profile = base_profile.clone();
    profile.cosim_models = generic_netlists(&converter.id).cosim_models;
    let mut bus = SystemBus::from_config(chip, &profile).expect("build onboarding bus");
    converter.power_up(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{tag}: both netlists bind to ADC channels {NTC_CHANNEL}/{DIODE_CHANNEL}"
    );

    let expected_diode_v = diode_load_line_voltage();
    let ntc_path = format!("adc.{}.{NTC_CHANNEL}_volts", converter.id);
    let diode_path = format!("adc.{}.{DIODE_CHANNEL}_volts", converter.id);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
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

        let [ntc_code, diode_code] = converter.convert_routed(&mut bus);
        assert_eq!(
            ntc_code,
            converter.expected_code(ntc_v),
            "{tag}: firmware-visible NTC conversion at {temperature_c} °C"
        );
        assert_eq!(
            diode_code,
            converter.expected_code(diode_v),
            "{tag}: firmware-visible diode conversion"
        );
        if let Some(idle) = converter.held_count(&mut bus, IDLE_CHANNEL) {
            assert_eq!(idle, 0xFFFF, "{tag}: unrouted channel stays undriven");
        }
        if matches!(
            converter.family,
            Family::Imxrt | Family::Sam4s | Family::Apollo4 | Family::Cc2538
        ) {
            assert_eq!(
                converter.convert(&mut bus, IDLE_CHANNEL),
                0,
                "{tag}: unrouted channel converts to zero"
            );
        }
        ntc_codes.push(ntc_code);
    }
    // The NTC sits on the low side: hotter means lower resistance, lower volts.
    if matches!(converter.family, Family::Apollo4) {
        // Above the 1.19 V reference the -10 and 25 °C dividers (2.79 V,
        // 1.65 V) both saturate at full scale; only 80 °C (0.37 V) is in range.
        assert_eq!(
            ntc_codes[..2],
            [4095, 4095],
            "{tag}: dividers above VREF saturate"
        );
        assert!(
            ntc_codes[2] < 4095,
            "{tag}: in-range divider resolves: {ntc_codes:?}"
        );
    } else {
        assert!(
            ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
            "{tag}: codes track the NTC temperature: {ntc_codes:?}"
        );
    }
}

#[test]
fn every_onboarding_profile_with_a_convertible_adc_routes_ntc_and_diode_into_each_instance() {
    let mut converted = Vec::new();
    let mut not_convertible = Vec::new();
    let mut without_adc = 0;
    let mut cpu_owned = 0;
    let mut instances = 0;
    for system_path in onboarding_profiles() {
        let board = system_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let profile =
            SystemManifest::from_file(&system_path).unwrap_or_else(|e| panic!("load {board}: {e}"));
        let chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
            .unwrap_or_else(|e| panic!("load {board} descriptor: {e}"));
        // The AVR ADC registers live on the CPU, not the bus; firmware
        // conversions for these are in `cosim_onboarding_avr_analog.rs`.
        if chip.peripherals.iter().any(|p| p.r#type == "avr_adc") {
            cpu_owned += 1;
            continue;
        }
        match converters(&chip) {
            Ok(list) if list.is_empty() => without_adc += 1,
            Ok(list) => {
                for converter in &list {
                    assert_converter_routes_generic_analog(&board, &chip, &profile, converter);
                }
                instances += list.len();
                converted.push(board);
            }
            Err(_) => not_convertible.push(board),
        }
    }
    assert_eq!(
        converted, CONVERTED,
        "onboarding profiles converted end to end"
    );
    assert_eq!(
        not_convertible, DECLARED_NOT_CONVERTIBLE,
        "onboarding profiles with an ADC LabWired cannot convert yet"
    );
    assert_eq!(
        instances, 39,
        "ADC instances exercised; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        cpu_owned, 34,
        "simavr profiles with a CPU-owned AVR ADC or comparator, driven in cosim_onboarding_avr_analog.rs"
    );
    assert_eq!(
        without_adc, 184,
        "onboarding profiles that declare no modelled ADC"
    );
}
