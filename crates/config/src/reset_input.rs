//! Descriptor-owned reset-pin electrical and pulse-width limits.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum ResetInputThresholds {
    Absolute { vil_max_v: f64, vih_min_v: f64 },
    Ratio { vil: f64, vih: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(try_from = "ResetInputYaml", into = "ResetInputYaml")]
pub struct ResetInputConfig {
    pub pin: String,
    pub active_low: bool,
    pub thresholds: ResetInputThresholds,
    pub min_assert_ns: u64,
    pub invalid_pulse_ns: u64,
    /// Nominal internal resistor for the circuit lowerer to stamp to this
    /// controller's actual VDD node. The reset state does not invent a rail.
    pub pull_up_ohms: Option<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetInputYaml {
    pin: String,
    active_low: bool,
    min_assert_ns: u64,
    invalid_pulse_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vil: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vih: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vil_max_v: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vih_min_v: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pull_up_ohms: Option<f64>,
}

impl TryFrom<ResetInputYaml> for ResetInputConfig {
    type Error = String;
    fn try_from(raw: ResetInputYaml) -> Result<Self, Self::Error> {
        if raw.pin.is_empty()
            || !raw
                .pin
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("reset_input.pin must name an explicit ASCII pin label".to_string());
        }
        if raw.invalid_pulse_ns > raw.min_assert_ns {
            return Err("reset_input.invalid_pulse_ns must not exceed min_assert_ns".to_string());
        }
        if raw.pull_up_ohms.is_some_and(|r| !r.is_finite() || r <= 0.0) {
            return Err("reset_input.pull_up_ohms must be finite and positive".to_string());
        }
        let thresholds = match (raw.vil, raw.vih, raw.vil_max_v, raw.vih_min_v) {
            (Some(vil), Some(vih), None, None) if vil.is_finite() && vih.is_finite() && 0.0 < vil && vil < vih && vih < 1.0 => ResetInputThresholds::Ratio { vil, vih },
            (None, None, Some(vil_max_v), Some(vih_min_v)) if vil_max_v.is_finite() && vih_min_v.is_finite() && 0.0 <= vil_max_v && vil_max_v < vih_min_v => ResetInputThresholds::Absolute { vil_max_v, vih_min_v },
            _ => return Err("reset_input requires exactly one valid absolute (vil_max_v/vih_min_v) or ratio (vil/vih) threshold pair".to_string()),
        };
        Ok(Self {
            pin: raw.pin,
            active_low: raw.active_low,
            thresholds,
            min_assert_ns: raw.min_assert_ns,
            invalid_pulse_ns: raw.invalid_pulse_ns,
            pull_up_ohms: raw.pull_up_ohms,
        })
    }
}

impl From<ResetInputConfig> for ResetInputYaml {
    fn from(config: ResetInputConfig) -> Self {
        let (vil, vih, vil_max_v, vih_min_v) = match config.thresholds {
            ResetInputThresholds::Absolute {
                vil_max_v,
                vih_min_v,
            } => (None, None, Some(vil_max_v), Some(vih_min_v)),
            ResetInputThresholds::Ratio { vil, vih } => (Some(vil), Some(vih), None, None),
        };
        Self {
            pin: config.pin,
            active_low: config.active_low,
            min_assert_ns: config.min_assert_ns,
            invalid_pulse_ns: config.invalid_pulse_ns,
            pull_up_ohms: config.pull_up_ohms,
            vil,
            vih,
            vil_max_v,
            vih_min_v,
        }
    }
}
