//! A level-sensitive external reset pin, separate from the VDD supervisor.
//! Thresholds and guaranteed valid/invalid pulse widths come from the chip
//! descriptor. An indeterminate-width pulse fails rather than gaining a label.
use labwired_config::{ResetInputConfig, ResetInputThresholds};

#[derive(Debug, Clone, Default)]
pub struct ResetInputState {
    pub(crate) config: Option<ResetInputConfig>,
    routed: bool,
    held: bool,
    pending_assert: bool,
    level_high: Option<bool>,
    pulse_qualified: bool,
    active_since_ns: Option<u64>,
    last_time_ns: u64,
    last_volts: Option<f64>,
    assertions: u64,
    io_qualified: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ResetInputStatus {
    pub pin: String,
    pub held: bool,
    pub volts: Option<f64>,
    pub assertions: u64,
    pub last_time_ns: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nominal_io_voltage_v: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_qualified: Option<bool>,
}

impl ResetInputState {
    pub fn new(config: Option<ResetInputConfig>) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }
    pub fn is_routed(&self) -> bool {
        self.routed
    }
    pub fn is_held(&self) -> bool {
        self.routed && self.held
    }
    pub fn attach(&mut self) {
        if self.routed || self.config.is_none() {
            return;
        }
        self.routed = true;
        // No instruction may precede the first valid reset-pin observation.
        self.held = true;
        // Attaching a route is not an electrical reset pulse. Construction
        // already reset the CPU; an initial high must not add a reset cause.
    }
    pub fn take_assertion(&mut self) -> bool {
        std::mem::take(&mut self.pending_assert)
    }
    pub fn status(&self) -> Option<ResetInputStatus> {
        (self.routed).then(|| ResetInputStatus {
            pin: self.config.as_ref().unwrap().pin.clone(),
            held: self.held,
            volts: self.last_volts,
            assertions: self.assertions,
            last_time_ns: self.last_time_ns,
            io_domain: self.config.as_ref().unwrap().io_domain.clone(),
            nominal_io_voltage_v: self.config.as_ref().unwrap().nominal_io_voltage_v,
            io_qualified: self.io_qualified,
        })
    }
    /// An unqualified receiver supply has no valid logic observation. Keep
    /// execution held without turning an unknown interval into a low pulse.
    pub fn hold_unqualified(&mut self, volts: f64, time_ns: u64) -> Result<(), String> {
        if !self.routed || !volts.is_finite() || time_ns < self.last_time_ns {
            return Err(
                "unqualified reset observation requires a routed finite monotonic sample".into(),
            );
        }
        self.held = true;
        self.io_qualified = Some(false);
        self.level_high = None;
        self.active_since_ns = None;
        self.pulse_qualified = false;
        self.last_time_ns = time_ns;
        self.last_volts = Some(volts);
        Ok(())
    }
    pub fn apply(&mut self, volts: f64, vdd: Option<f64>, time_ns: u64) -> Result<(), String> {
        let Some(config) = self.config.as_ref() else {
            return Err("reset_input metadata is absent".to_string());
        };
        if !self.routed || !volts.is_finite() || time_ns < self.last_time_ns {
            return Err(
                "reset input requires a routed finite voltage on a monotonic timeline".to_string(),
            );
        }
        if config.io_domain.is_some() {
            self.io_qualified = Some(true);
        }
        let (vil, vih) = match config.thresholds {
            ResetInputThresholds::Absolute {
                vil_max_v,
                vih_min_v,
            } => (vil_max_v, vih_min_v),
            ResetInputThresholds::Ratio { vil, vih } => {
                let vdd = vdd
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .ok_or("reset input ratio thresholds require a known VDD")?;
                if vdd == 0.0 {
                    self.level_high = Some(false);
                }
                (vil * vdd, vih * vdd)
            }
        };
        // Zero VDD has no valid logic high for ratio thresholds.
        if vih > 0.0 {
            if volts <= vil {
                self.level_high = Some(false);
            } else if volts >= vih {
                self.level_high = Some(true);
            }
        }
        let Some(level_high) = self.level_high else {
            // An initial observation in the Schmitt band has no known level.
            // Keep the construction hold without inventing a low pulse.
            self.last_time_ns = time_ns;
            self.last_volts = Some(volts);
            return Ok(());
        };
        let active = if config.active_low {
            !level_high
        } else {
            level_high
        };
        if active {
            let start = *self.active_since_ns.get_or_insert(time_ns);
            if !self.pulse_qualified && time_ns - start >= config.min_assert_ns {
                self.held = true;
                self.pulse_qualified = true;
                self.pending_assert = true;
                self.assertions += 1;
            }
        } else {
            if let Some(start) = self.active_since_ns.take() {
                let width = time_ns - start;
                if !self.pulse_qualified {
                    if width >= config.min_assert_ns {
                        // A whole valid pulse can fit between observations.
                        // Reset once at its release even if no active sample
                        // landed exactly on the qualification boundary.
                        self.pending_assert = true;
                        self.assertions += 1;
                    } else if width > config.invalid_pulse_ns {
                        return Err(format!(
                            "reset pin '{}' pulse {width} ns is in the indeterminate {}..{} ns band",
                            config.pin, config.invalid_pulse_ns, config.min_assert_ns
                        ));
                    }
                }
            }
            self.pulse_qualified = false;
            self.held = false;
        }
        self.last_time_ns = time_ns;
        self.last_volts = Some(volts);
        Ok(())
    }
}
