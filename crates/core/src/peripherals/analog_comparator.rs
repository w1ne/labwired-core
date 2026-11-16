// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A voltage comparator a chip declares by data: which pads (or internal
//! references) each input selector reaches, and which EXTI line the output
//! drives. The register front end is a named `profile`; everything the
//! comparator compares comes from the chip descriptor, so a second part with
//! the same register file but other pads needs only YAML.
//!
//! Inputs are analog pad levels in millivolts, driven through
//! [`SystemBus::set_analog_pad_mv`](crate::bus::SystemBus::set_analog_pad_mv)
//! (and the co-simulation `analog:<pad>` route). A pad nobody has driven
//! reads 0 mV. The output is evaluated again on every register write and
//! every input change; a change of the output after polarity is an edge on
//! the configured EXTI line, which the EXTI then latches by its own
//! RTSR/FTSR selection.
//!
//! ## `profile: at32f421`
//!
//! RM_AT32F421 V2.05 §19.6: CMP_CTRLSTS at +0x00 (absolute 0x4001_001C,
//! reset 0x0000_0080), G_FILTER_EN +0x08, HIGH_PULSE +0x0C, LOW_PULSE +0x10.
//! Modelled: CMPEN, CMPINVSEL, CMPNINVSEL, CMPP, CMPSSEL (stored; the
//! response time of each speed is not modelled), SCALEN/BRGEN on the four
//! VREFINT taps (§19.6.1: SCALEN=0 puts 0 V on all four, SCALEN=1 BRGEN=0
//! 1.2 V on all four, both set 1.2/0.9/0.6/0.3 V), the write-once CMPWP lock
//! and the read-only CMPVALUE (the output after polarity, which is also what
//! reaches EXINT21, §19.3). Refused rather than ignored: hysteresis (the
//! datasheet thresholds are not transcribed), blanking, output redirection
//! to timers (CMPTAG), the CMPIS switch and the glitch filter.

use crate::{Peripheral, SimResult, SimulationError};

/// One comparator input as the descriptor names it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
enum Source {
    /// An analog pad, by label (`PA1`).
    Pad(String),
    /// Analog ground.
    Ground,
    /// Internal reference tap `quarters`/4 of VREFINT, behind the scaler.
    Vrefint { quarters: u8 },
}

impl Source {
    fn parse(name: &str) -> anyhow::Result<Self> {
        Ok(match name.to_ascii_uppercase().as_str() {
            "VSSA" | "GND" => Self::Ground,
            "VREFINT" => Self::Vrefint { quarters: 4 },
            "VREFINT_3_4" => Self::Vrefint { quarters: 3 },
            "VREFINT_1_2" => Self::Vrefint { quarters: 2 },
            "VREFINT_1_4" => Self::Vrefint { quarters: 1 },
            pad if pad.starts_with('P') && pad.len() >= 3 => Self::Pad(pad.to_string()),
            other => anyhow::bail!("analog_comparator: unknown input '{other}'"),
        })
    }
}

/// Register front end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ComparatorProfile {
    At32F421,
}

const AT32_CTRLSTS_RESET: u32 = 0x0000_0080;
const AT32_WP: u32 = 1 << 31;
const AT32_VALUE: u32 = 1 << 30;
/// Every writable CTRLSTS bit (reserved 29:24, 21, 14:13 and 9 excluded).
const AT32_CTRLSTS_RW: u32 = 0x80df_9dff;
/// The fields this model refuses to set: CMPBLANKING (20:18), CMPHYST
/// (17:16), CMPTAG (12:10), CMPIS (1).
const AT32_UNMODELLED: u32 = (0x7 << 18) | (0x3 << 16) | (0x7 << 10) | (1 << 1);

#[derive(Debug, serde::Serialize)]
pub struct AnalogComparator {
    profile: ComparatorProfile,
    ctrlsts: u32,
    filter_en: u32,
    high_pulse: u32,
    low_pulse: u32,
    plus: Vec<Source>,
    minus: Vec<Source>,
    vrefint_mv: f64,
    /// Driven pad levels, millivolts; absent = 0 mV.
    pads: std::collections::BTreeMap<String, u16>,
    /// Output after polarity (CMPVALUE).
    output: bool,
    exti_line: Option<u8>,
    #[serde(skip)]
    edges: Vec<(u8, bool, bool)>,
}

impl AnalogComparator {
    /// `plus` / `minus` are indexed by the profile's selector field value.
    pub fn new(
        profile: ComparatorProfile,
        plus: &[String],
        minus: &[String],
        vrefint_mv: f64,
        exti_line: Option<u8>,
    ) -> anyhow::Result<Self> {
        let (plus_len, minus_len) = match profile {
            ComparatorProfile::At32F421 => (4, 8),
        };
        anyhow::ensure!(
            plus.len() == plus_len && minus.len() == minus_len,
            "analog_comparator: profile needs {plus_len} plus and {minus_len} minus inputs, got {} and {}",
            plus.len(),
            minus.len()
        );
        anyhow::ensure!(
            vrefint_mv.is_finite() && vrefint_mv > 0.0,
            "analog_comparator: vrefint_mv must be positive"
        );
        let mut c = Self {
            profile,
            ctrlsts: AT32_CTRLSTS_RESET,
            filter_en: 0,
            high_pulse: 0,
            low_pulse: 0,
            plus: plus
                .iter()
                .map(|s| Source::parse(s))
                .collect::<anyhow::Result<_>>()?,
            minus: minus
                .iter()
                .map(|s| Source::parse(s))
                .collect::<anyhow::Result<_>>()?,
            vrefint_mv,
            pads: Default::default(),
            output: false,
            exti_line,
            edges: Vec::new(),
        };
        c.output = c.evaluate();
        Ok(c)
    }

    /// The level on `source` in millivolts under the current scaler setting.
    fn level_mv(&self, source: &Source) -> f64 {
        match source {
            Source::Ground => 0.0,
            Source::Pad(pad) => f64::from(self.pads.get(pad).copied().unwrap_or(0)),
            Source::Vrefint { quarters } => {
                let scalen = self.ctrlsts & (1 << 23) != 0;
                let brgen = self.ctrlsts & (1 << 22) != 0;
                match (scalen, brgen) {
                    (false, _) => 0.0,
                    (true, false) => self.vrefint_mv,
                    (true, true) => self.vrefint_mv * f64::from(*quarters) / 4.0,
                }
            }
        }
    }

    /// The output after polarity for the current inputs and configuration.
    fn evaluate(&self) -> bool {
        if self.ctrlsts & 1 == 0 {
            return false;
        }
        let plus = &self.plus[((self.ctrlsts >> 7) & 0x3) as usize];
        let minus = &self.minus[((self.ctrlsts >> 4) & 0x7) as usize];
        let raw = self.level_mv(plus) > self.level_mv(minus);
        raw ^ (self.ctrlsts & (1 << 15) != 0)
    }

    fn update_output(&mut self) {
        let next = self.evaluate();
        if next != self.output {
            if let Some(line) = self.exti_line {
                self.edges.push((line, self.output, next));
            }
            self.output = next;
        }
    }

    /// CMPVALUE as firmware reads it.
    pub fn output(&self) -> bool {
        self.output
    }

    fn read_reg(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            0x00 => (self.ctrlsts & !AT32_VALUE) | (u32::from(self.output) << 30),
            0x08 => self.filter_en,
            0x0c => self.high_pulse,
            0x10 => self.low_pulse,
            _ => return Err(no_register(offset)),
        })
    }

    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            0x00 => {
                if self.ctrlsts & AT32_WP != 0 {
                    // CMPWP: read-only until a system reset (§19.4).
                    return Ok(());
                }
                if value & AT32_UNMODELLED != 0 {
                    return Err(SimulationError::Other(format!(
                        "AT32F421 CMP: CTRLSTS {:#x} selects hysteresis, blanking, a timer \
                         output target or CMPIS, which are not modelled",
                        value & AT32_UNMODELLED
                    )));
                }
                self.ctrlsts = value & AT32_CTRLSTS_RW;
            }
            0x08 => {
                if value & 1 != 0 {
                    return Err(SimulationError::Other(
                        "AT32F421 CMP: the glitch filter (G_FILTER_EN.GFE) is not modelled".into(),
                    ));
                }
                self.filter_en = 0;
            }
            0x0c => self.high_pulse = value & 0x3f,
            0x10 => self.low_pulse = value & 0x3f,
            _ => return Err(no_register(offset)),
        }
        self.update_output();
        Ok(())
    }

    fn reset_registers(&mut self) {
        self.ctrlsts = AT32_CTRLSTS_RESET;
        self.filter_en = 0;
        self.high_pulse = 0;
        self.low_pulse = 0;
        self.update_output();
    }
}

fn no_register(offset: u64) -> SimulationError {
    SimulationError::Other(format!("AT32F421 CMP: no register at +{offset:#x}"))
}

impl Peripheral for AnalogComparator {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_reg(offset & !3)?;
        Ok((word >> ((offset % 4) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let word = self.read_reg(reg)? & !AT32_VALUE;
        let word = (word & !(0xff << shift)) | (u32::from(value) << shift);
        self.write_reg(reg, word)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.read_reg(offset & !3)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value)
    }

    fn set_analog_pad_input(&mut self, pad: &str, millivolts: u16) -> bool {
        let pad = pad.to_ascii_uppercase();
        let wired = self
            .plus
            .iter()
            .chain(&self.minus)
            .any(|s| matches!(s, Source::Pad(p) if *p == pad));
        if !wired {
            return false;
        }
        self.pads.insert(pad, millivolts);
        self.update_output();
        true
    }

    fn drives_exti_lines(&self) -> bool {
        self.exti_line.is_some()
    }

    fn take_exti_line_edges(&mut self, out: &mut Vec<(u8, bool, bool)>) {
        out.append(&mut self.edges);
    }

    /// Power-on and system reset return the registers (and the CMPWP lock)
    /// to reset state; driven pad levels are outside the chip and stay.
    fn on_node_reset(&mut self) {
        self.reset_registers();
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    /// A register bank evaluated on writes and input changes: nothing to tick.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at32() -> AnalogComparator {
        let plus = ["PA5", "PA1", "PA0", "VSSA"].map(String::from);
        let minus = [
            "VREFINT_1_4",
            "VREFINT_1_2",
            "VREFINT_3_4",
            "VREFINT",
            "PA4",
            "PA5",
            "PA0",
            "PA2",
        ]
        .map(String::from);
        AnalogComparator::new(ComparatorProfile::At32F421, &plus, &minus, 1200.0, Some(21)).unwrap()
    }

    fn edges(c: &mut AnalogComparator) -> Vec<(u8, bool, bool)> {
        let mut out = Vec::new();
        c.take_exti_line_edges(&mut out);
        out
    }

    #[test]
    fn reset_value_and_disabled_output() {
        let mut c = at32();
        assert_eq!(c.read_u32(0).unwrap(), 0x80, "CMPNINVSEL=01 (PA1) at reset");
        c.set_analog_pad_input("PA1", 3000);
        assert_eq!(c.read_u32(0).unwrap() & AT32_VALUE, 0, "disabled reads 0");
        assert!(edges(&mut c).is_empty());
    }

    #[test]
    fn selected_pads_are_compared_and_edges_reach_the_exti_line() {
        let mut c = at32();
        // AM32 COMP_PA0: CMPEN, CMPINVSEL=110 (PA0), CMPNINVSEL=00 (PA5).
        c.set_analog_pad_input("PA5", 1000);
        c.set_analog_pad_input("PA0", 1500);
        c.write_u32(0, 0b110_0001).unwrap();
        assert!(!c.output());
        c.set_analog_pad_input("PA5", 1600);
        assert_ne!(c.read_u32(0).unwrap() & AT32_VALUE, 0);
        c.set_analog_pad_input("PA5", 1400);
        assert_eq!(edges(&mut c), vec![(21, false, true), (21, true, false)]);
        // A pad no selector reaches is not taken.
        assert!(!c.set_analog_pad_input("PB0", 100));
    }

    #[test]
    fn polarity_inverts_and_switching_the_input_is_an_edge() {
        let mut c = at32();
        c.set_analog_pad_input("PA5", 2000);
        c.set_analog_pad_input("PA4", 1000);
        c.set_analog_pad_input("PA0", 3000);
        c.write_u32(0, 0b100_0001).unwrap(); // PA5 vs PA4: high
        assert!(c.output());
        c.write_u32(0, 0b110_0001).unwrap(); // PA5 vs PA0: low
        assert!(!c.output());
        c.write_u32(0, 0b110_0001 | (1 << 15)).unwrap(); // inverted: high
        assert!(c.output());
        assert_eq!(
            edges(&mut c),
            vec![(21, false, true), (21, true, false), (21, false, true)]
        );
    }

    #[test]
    fn vrefint_taps_follow_scalen_and_brgen() {
        let mut c = at32();
        c.set_analog_pad_input("PA1", 500);
        let sel = |inv: u32, extra: u32| 1 | (1 << 7) | (inv << 4) | extra;
        c.write_u32(0, sel(0, 0)).unwrap(); // SCALEN=0: taps at 0 V
        assert!(c.output());
        c.write_u32(0, sel(0, 1 << 23)).unwrap(); // SCALEN only: 1.2 V
        assert!(!c.output());
        c.write_u32(0, sel(0, (1 << 23) | (1 << 22))).unwrap(); // 1/4: 0.3 V
        assert!(c.output());
        c.write_u32(0, sel(1, (1 << 23) | (1 << 22))).unwrap(); // 1/2: 0.6 V
        assert!(!c.output());
    }

    #[test]
    fn write_protect_latches_until_reset() {
        let mut c = at32();
        c.write_u32(0, AT32_WP | 1).unwrap();
        c.write_u32(0, 0).unwrap();
        assert_eq!(c.read_u32(0).unwrap() & (AT32_WP | 1), AT32_WP | 1);
        c.on_system_reset();
        assert_eq!(c.read_u32(0).unwrap(), 0x80);
    }

    #[test]
    fn unmodelled_fields_and_holes_refuse() {
        let mut c = at32();
        assert!(c.write_u32(0, 1 | (1 << 16)).is_err(), "hysteresis");
        assert!(c.write_u32(0, 1 | (3 << 18)).is_err(), "blanking");
        assert!(c.write_u32(0, 1 | (6 << 10)).is_err(), "CMPTAG");
        assert!(c.write_u32(0x08, 1).is_err(), "glitch filter");
        assert!(c.read_u32(0x04).is_err(), "reserved word");
        assert_eq!(
            c.read_u32(0).unwrap(),
            0x80,
            "a refused write changes nothing"
        );
    }
}
