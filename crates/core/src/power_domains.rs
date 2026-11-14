//! Sampled MCU power-domain observations and reset supervision.
//!
//! Physical operating qualification, detector reset state and a nominal I/O
//! threshold calibration are separate facts. Timers advance only when a
//! sampled source applies an explicit simulation boundary.
use labwired_config::{PowerDomainConfig, PowerDomainsConfig};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

const MAX_PENDING_CONTROLS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerResetCause {
    PowerOn,
    BrownOut,
    ExternalPin,
    System,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PowerDomainObservation {
    pub measured_mode: bool,
    pub routed: bool,
    pub voltage_v: Option<f64>,
    pub nominal_v: f64,
    pub min_v: f64,
    pub max_v: f64,
    pub within_operating_range: Option<bool>,
    pub observed_time_ns: Option<u64>,
}
#[derive(Debug, Clone)]
struct RailState {
    config: PowerDomainConfig,
    routed: bool,
    voltage_v: Option<f64>,
    observed_time_ns: Option<u64>,
}
#[derive(Debug, Default, Clone)]
struct Registry {
    measured_mode: bool,
    rails: BTreeMap<String, RailState>,
}

/// A peripheral's read-only view of one descriptor-declared physical rail.
#[derive(Debug, Clone)]
pub struct PowerDomainHandle {
    domain: String,
    registry: Arc<Mutex<Registry>>,
}
impl PowerDomainHandle {
    pub fn observation(&self) -> PowerDomainObservation {
        let registry = self
            .registry
            .lock()
            .expect("power domain registry poisoned");
        let rail = &registry.rails[&self.domain];
        PowerDomainObservation {
            measured_mode: registry.measured_mode,
            routed: rail.routed,
            voltage_v: rail.voltage_v,
            nominal_v: rail.config.nominal_v,
            min_v: rail.config.min_v,
            max_v: rail.config.max_v,
            within_operating_range: rail
                .voltage_v
                .map(|v| rail.config.min_v <= v && v <= rail.config.max_v),
            observed_time_ns: rail.observed_time_ns,
        }
    }
    pub fn is_available(&self) -> bool {
        let observation = self.observation();
        !observation.measured_mode || observation.within_operating_range == Some(true)
    }
    /// An unrouted registry explicitly assumes its declared nominal supply.
    /// Once ANY peer rail is routed, unknown observations remain unknown.
    pub fn effective_voltage_v(&self) -> Option<f64> {
        let observation = self.observation();
        if observation.measured_mode {
            observation.voltage_v
        } else {
            Some(observation.nominal_v)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct BrownOutConfiguration {
    pub enabled: bool,
    pub falling_v: f64,
    pub rising_v: f64,
}
#[derive(Debug, Clone)]
struct ControlState {
    sampled_source: bool,
    accepted: BrownOutConfiguration,
    pending: VecDeque<BrownOutConfiguration>,
}
/// The exact peripheral owns register encodings. The generic supervisor owns
/// accepted voltages and sampled delays; it never embeds a chip's VSEL table.
#[derive(Debug, Clone)]
pub struct PowerSupervisorControl {
    inner: Arc<Mutex<ControlState>>,
}
impl PowerSupervisorControl {
    pub fn configure_brown_out(
        &self,
        enabled: bool,
        falling_v: f64,
        rising_v: f64,
    ) -> Result<(), String> {
        if !(falling_v.is_finite()
            && rising_v.is_finite()
            && 0.0 < falling_v
            && falling_v <= rising_v)
        {
            return Err("brown-out control requires finite 0 < falling_v <= rising_v".into());
        }
        let next = BrownOutConfiguration {
            enabled,
            falling_v,
            rising_v,
        };
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "power supervisor control poisoned")?;
        if state.accepted == next {
            return Ok(());
        }
        if !state.sampled_source {
            return Err(
                "state-changing brown-out control requires an attached sampled supervision source"
                    .into(),
            );
        }
        if state.pending.len() >= MAX_PENDING_CONTROLS {
            return Err("brown-out control queue is full; no request was accepted".into());
        }
        state.pending.push_back(next);
        state.accepted = next;
        Ok(())
    }
    pub fn brown_out_configuration(&self) -> BrownOutConfiguration {
        self.inner
            .lock()
            .expect("power supervisor control poisoned")
            .accepted
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DomainBrownOutStatus {
    pub accepted: BrownOutConfiguration,
    pub applied: BrownOutConfiguration,
    pub pending_changes: usize,
    pub active_at_ns: Option<u64>,
    pub held_in_reset: bool,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct NominalInputThresholdCalibration {
    pub nominal_io_voltage_v: f64,
    pub minimum_io_voltage_v: Option<f64>,
    pub currently_qualified: bool,
    pub vil_max_v: f64,
    pub vih_min_v: f64,
    pub scope: &'static str,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PowerDomainStatus {
    pub measured_mode: bool,
    pub rails: BTreeMap<String, PowerDomainObservation>,
    pub io_domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_threshold_calibration: Option<NominalInputThresholdCalibration>,
    pub execution_domains: Vec<String>,
    pub execution_qualified: bool,
    pub supervision_qualified: bool,
    pub held_in_reset: bool,
    pub por_latched: bool,
    pub restart_pending: bool,
    pub last_sample_ns: Option<u64>,
    pub sample_interval_ns: Option<u64>,
    pub timing: &'static str,
    pub last_reset_cause: Option<PowerResetCause>,
    pub reset_events: u64,
    pub brown_out: Option<DomainBrownOutStatus>,
}

#[derive(Debug, Default)]
pub struct McuPowerDomains {
    runtime: Option<Box<DomainRuntime>>,
}
#[derive(Debug, Clone)]
struct DomainRuntime {
    config: PowerDomainsConfig,
    registry: Arc<Mutex<Registry>>,
    control: Option<PowerSupervisorControl>,
    bod_config: Option<BrownOutConfiguration>,
    last_sample_ns: Option<u64>,
    sample_interval_ns: Option<u64>,
    por_latched: bool,
    por_since_ns: Option<u64>,
    por_event_sent: bool,
    bod_held: bool,
    bod_active_at_ns: Option<u64>,
    bod_assert_since_ns: Option<u64>,
    bod_release_since_ns: Option<u64>,
    reset_delay_pending: Option<u64>,
    events: VecDeque<PowerResetCause>,
    restart_pending: bool,
    last_reset_cause: Option<PowerResetCause>,
    reset_events: u64,
}

impl McuPowerDomains {
    pub fn new(config: Option<PowerDomainsConfig>) -> Result<Self, String> {
        let Some(config) = config else {
            return Ok(Self::default());
        };
        config.validate()?;
        let rails = config
            .rails
            .iter()
            .map(|(name, config)| {
                (
                    name.clone(),
                    RailState {
                        config: config.clone(),
                        routed: false,
                        voltage_v: None,
                        observed_time_ns: None,
                    },
                )
            })
            .collect();
        let bod_config = config.brown_out.as_ref().map(|bod| BrownOutConfiguration {
            enabled: bod.enabled,
            falling_v: bod.falling_v,
            rising_v: bod.rising_v,
        });
        let control = bod_config.map(|accepted| PowerSupervisorControl {
            inner: Arc::new(Mutex::new(ControlState {
                sampled_source: false,
                accepted,
                pending: VecDeque::new(),
            })),
        });
        Ok(Self {
            runtime: Some(Box::new(DomainRuntime {
                config,
                registry: Arc::new(Mutex::new(Registry {
                    measured_mode: false,
                    rails,
                })),
                control,
                bod_config,
                last_sample_ns: None,
                sample_interval_ns: None,
                por_latched: true,
                por_since_ns: None,
                por_event_sent: false,
                bod_held: false,
                bod_active_at_ns: None,
                bod_assert_since_ns: None,
                bod_release_since_ns: None,
                reset_delay_pending: None,
                events: VecDeque::new(),
                restart_pending: false,
                last_reset_cause: None,
                reset_events: 0,
            })),
        })
    }
    pub fn is_configured(&self) -> bool {
        self.runtime.is_some()
    }
    pub fn is_measured(&self) -> bool {
        self.runtime.as_ref().is_some_and(|r| {
            r.registry
                .lock()
                .expect("power domain registry poisoned")
                .measured_mode
        })
    }
    pub fn handle(&self, domain: &str) -> Option<PowerDomainHandle> {
        let runtime = self.runtime.as_ref()?;
        runtime
            .config
            .rails
            .contains_key(domain)
            .then(|| PowerDomainHandle {
                domain: domain.into(),
                registry: runtime.registry.clone(),
            })
    }
    pub fn io_handle(&self) -> Option<PowerDomainHandle> {
        self.handle(&self.runtime.as_ref()?.config.io_domain)
    }
    pub fn supervisor_control(&self) -> Option<PowerSupervisorControl> {
        self.runtime.as_ref()?.control.clone()
    }
    pub fn validate_route(&self, domain: &str) -> Result<(), String> {
        self.handle(domain)
            .map(|_| ())
            .ok_or_else(|| format!("MCU has no declared power domain '{domain}'"))
    }
    pub fn attach_sampled_routes(
        &mut self,
        domains: &[String],
        step_ns: u64,
    ) -> Result<(), String> {
        if domains.is_empty() {
            return Ok(());
        }
        if step_ns == 0 {
            return Err("MCU power domains require a positive sampled interval".into());
        }
        for domain in domains {
            self.validate_route(domain)?;
        }
        let runtime = self.runtime.as_mut().unwrap();
        if runtime
            .sample_interval_ns
            .is_some_and(|previous| previous != step_ns)
        {
            return Err(
                "one MCU power registry cannot attach conflicting sampled intervals".into(),
            );
        }
        let mut registry = runtime
            .registry
            .lock()
            .map_err(|_| "power domain registry poisoned")?;
        if !registry.measured_mode {
            runtime.por_latched = runtime.config.latched_por.is_none();
            runtime.por_since_ns = None;
            runtime.por_event_sent = false;
            runtime.bod_held = false;
        }
        registry.measured_mode = true;
        for domain in domains {
            registry.rails.get_mut(domain).unwrap().routed = true;
        }
        runtime.sample_interval_ns = Some(step_ns);
        if let Some(control) = &runtime.control {
            control
                .inner
                .lock()
                .map_err(|_| "power supervisor control poisoned")?
                .sampled_source = true;
        }
        Ok(())
    }
    /// Validate the entire rail batch before changing any observation.
    /// Every configured timer is then sampled once at the supplied time.
    pub fn apply_voltages(
        &mut self,
        updates: &[(String, f64)],
        time_ns: u64,
    ) -> Result<(), String> {
        let Some(runtime) = self.runtime.as_mut() else {
            return if updates.is_empty() {
                Ok(())
            } else {
                Err("MCU power domains are not configured".into())
            };
        };
        if runtime
            .last_sample_ns
            .is_some_and(|previous| time_ns < previous)
        {
            return Err("MCU power observations must use a monotonic simulation timeline".into());
        }

        // Hold both live locks through preparation and publication. Existing
        // peripheral handles keep their Arcs, and concurrent control writes
        // cannot be lost when a staged transition commits.
        let registry_arc = Arc::clone(&runtime.registry);
        let control_arc = runtime
            .control
            .as_ref()
            .map(|control| Arc::clone(&control.inner));
        let mut registry = registry_arc
            .lock()
            .map_err(|_| "power domain registry poisoned")?;
        let mut control = control_arc
            .as_ref()
            .map(|arc| arc.lock().map_err(|_| "power supervisor control poisoned"))
            .transpose()?;
        if !registry.measured_mode {
            return if updates.is_empty() {
                Ok(())
            } else {
                Err("MCU rail observations require an attached sampled source".into())
            };
        }
        let mut seen = BTreeSet::new();
        for (domain, volts) in updates {
            let rail = registry
                .rails
                .get(domain)
                .ok_or_else(|| format!("MCU has no declared power domain '{domain}'"))?;
            if !rail.routed || !seen.insert(domain) || !volts.is_finite() || *volts < 0.0 {
                return Err(format!("power domain '{domain}' requires one routed finite nonnegative voltage per boundary"));
            }
        }
        let mut staged = (**runtime).clone();
        staged.registry = Arc::new(Mutex::new(registry.clone()));
        staged.control = control.as_ref().map(|state| PowerSupervisorControl {
            inner: Arc::new(Mutex::new((**state).clone())),
        });
        {
            let mut staged_registry = staged
                .registry
                .lock()
                .map_err(|_| "staged power registry poisoned")?;
            for (domain, volts) in updates {
                let rail = staged_registry.rails.get_mut(domain).unwrap();
                rail.voltage_v = Some(*volts);
                rail.observed_time_ns = Some(time_ns);
            }
        }
        staged.last_sample_ns = Some(time_ns);
        staged.advance(time_ns)?;

        let next_registry = staged
            .registry
            .lock()
            .map_err(|_| "staged power registry poisoned")?
            .clone();
        let next_control = staged
            .control
            .as_ref()
            .map(|value| {
                value
                    .inner
                    .lock()
                    .map(|state| state.clone())
                    .map_err(|_| "staged supervisor control poisoned")
            })
            .transpose()?;
        *registry = next_registry;
        if let (Some(live), Some(next)) = (control.as_mut(), next_control) {
            **live = next;
        }
        staged.registry = Arc::clone(&registry_arc);
        staged.control = control_arc.as_ref().map(|inner| PowerSupervisorControl {
            inner: Arc::clone(inner),
        });
        **runtime = staged;
        Ok(())
    }
    pub fn is_held(&self) -> bool {
        self.is_measured()
            && self
                .runtime
                .as_ref()
                .is_some_and(|r| !r.por_latched || r.bod_held)
    }
    pub fn execution_qualified(&self) -> bool {
        self.runtime
            .as_ref()
            .is_none_or(|r| r.execution_qualified())
    }
    pub fn blocks_execution(&self) -> bool {
        self.is_held()
            || !self.execution_qualified()
            || self.runtime.as_ref().is_some_and(|r| r.restart_pending)
    }
    pub fn system_reset_scope(&self) -> Option<labwired_config::PowerSystemResetScope> {
        Some(self.runtime.as_ref()?.config.system_reset_scope)
    }
    /// A RUN/system reset resets detector configuration once. Timing begins
    /// at the next sampled boundary, so host call chunking cannot change it.
    pub fn record_reset(&mut self, cause: PowerResetCause) -> Result<(), String> {
        let Some(runtime) = self.runtime.as_mut() else {
            return Ok(());
        };
        runtime.enqueue_reset(cause)?;
        let delay = runtime
            .config
            .brown_out
            .as_ref()
            .map(|bod| bod.enable_delay_ns);
        runtime.restore_brown_out_defaults();
        runtime.reset_delay_pending = delay;
        Ok(())
    }
    pub fn take_reset_event(&mut self) -> Option<PowerResetCause> {
        self.runtime.as_mut()?.events.pop_front()
    }
    pub fn needs_restart(&self) -> bool {
        self.runtime.as_ref().is_some_and(|r| r.restart_pending)
            && !self.is_held()
            && self.execution_qualified()
    }
    pub fn finish_restart(&mut self) {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.restart_pending = false;
        }
    }
    pub fn status(&self) -> Option<PowerDomainStatus> {
        let runtime = self.runtime.as_ref()?;
        let rails = runtime
            .config
            .rails
            .keys()
            .map(|name| (name.clone(), self.handle(name).unwrap().observation()))
            .collect();
        let brown_out = runtime.control.as_ref().map(|control| {
            let state = control
                .inner
                .lock()
                .expect("power supervisor control poisoned");
            DomainBrownOutStatus {
                accepted: state.accepted,
                applied: runtime.bod_config.unwrap(),
                pending_changes: state.pending.len(),
                active_at_ns: runtime.bod_active_at_ns,
                held_in_reset: runtime.bod_held,
            }
        });
        Some(PowerDomainStatus {
            measured_mode: self.is_measured(),
            rails,
            io_domain: runtime.config.io_domain.clone(),
            input_threshold_calibration: None,
            execution_domains: runtime.config.execution_domains.clone(),
            execution_qualified: runtime.execution_qualified(),
            supervision_qualified: runtime.supervision_qualified(),
            held_in_reset: self.is_held(),
            por_latched: runtime.por_latched,
            restart_pending: runtime.restart_pending,
            last_sample_ns: runtime.last_sample_ns,
            sample_interval_ns: runtime.sample_interval_ns,
            timing: "sample-boundary-quantized",
            last_reset_cause: runtime.last_reset_cause,
            reset_events: runtime.reset_events,
            brown_out,
        })
    }
}

impl DomainRuntime {
    fn voltage(&self, domain: &str) -> Option<f64> {
        let registry = self
            .registry
            .lock()
            .expect("power domain registry poisoned");
        let rail = &registry.rails[domain];
        if registry.measured_mode {
            rail.voltage_v
        } else {
            Some(rail.config.nominal_v)
        }
    }
    fn available(&self, domain: &str) -> bool {
        let registry = self
            .registry
            .lock()
            .expect("power domain registry poisoned");
        let rail = &registry.rails[domain];
        !registry.measured_mode
            || rail
                .voltage_v
                .is_some_and(|v| rail.config.min_v <= v && v <= rail.config.max_v)
    }
    fn supervision_qualified(&self) -> bool {
        self.config
            .latched_por
            .as_ref()
            .is_none_or(|por| self.available(&por.detector_domain))
            && self
                .config
                .brown_out
                .as_ref()
                .is_none_or(|bod| self.available(&bod.detector_domain))
    }
    fn execution_qualified(&self) -> bool {
        self.config
            .execution_domains
            .iter()
            .all(|domain| self.available(domain))
            && self.supervision_qualified()
    }
    fn enqueue_reset(&mut self, cause: PowerResetCause) -> Result<(), String> {
        if self.events.len() >= MAX_PENDING_CONTROLS {
            return Err("MCU reset event queue is full".into());
        }
        self.events.push_back(cause);
        self.restart_pending = true;
        self.last_reset_cause = Some(cause);
        self.reset_events = self.reset_events.saturating_add(1);
        Ok(())
    }
    fn restore_brown_out_defaults(&mut self) {
        let Some(bod) = &self.config.brown_out else {
            return;
        };
        let defaults = BrownOutConfiguration {
            enabled: bod.enabled,
            falling_v: bod.falling_v,
            rising_v: bod.rising_v,
        };
        self.bod_config = Some(defaults);
        self.bod_assert_since_ns = None;
        self.bod_release_since_ns = None;
        if let Some(control) = &self.control {
            let mut state = control
                .inner
                .lock()
                .expect("power supervisor control poisoned");
            state.accepted = defaults;
            state.pending.clear();
        }
    }
    fn deadline(time_ns: u64, delay_ns: u64) -> Result<u64, String> {
        time_ns
            .checked_add(delay_ns)
            .ok_or_else(|| "MCU supervisor time overflow".into())
    }
    fn advance(&mut self, time_ns: u64) -> Result<(), String> {
        if let Some(delay) = self.reset_delay_pending.take() {
            self.bod_active_at_ns = Some(Self::deadline(time_ns, delay)?);
        }
        self.apply_controls(time_ns)?;
        self.advance_por(time_ns)?;
        self.advance_brown_out(time_ns)
    }
    fn apply_controls(&mut self, time_ns: u64) -> Result<(), String> {
        let Some(control) = &self.control else {
            return Ok(());
        };
        let pending: Vec<_> = control
            .inner
            .lock()
            .map_err(|_| "power supervisor control poisoned")?
            .pending
            .drain(..)
            .collect();
        for next in pending {
            let previous = self.bod_config.unwrap();
            let bod = self.config.brown_out.as_ref().unwrap();
            let delay = if !previous.enabled && next.enabled {
                bod.enable_delay_ns
            } else {
                bod.reprogram_delay_ns
            };
            self.bod_active_at_ns = Some(Self::deadline(time_ns, delay)?);
            self.bod_assert_since_ns = None;
            self.bod_release_since_ns = None;
            self.bod_config = Some(next);
            if !next.enabled {
                self.bod_held = false;
            }
        }
        Ok(())
    }
    fn advance_por(&mut self, time_ns: u64) -> Result<(), String> {
        let Some(por) = &self.config.latched_por else {
            return Ok(());
        };
        let rearm = self
            .voltage(&por.detector_domain)
            .is_some_and(|v| v < por.rearm_below_v);
        let qualifies = self.available(&por.detector_domain)
            && self
                .voltage(&por.sense_domain)
                .is_some_and(|v| v >= por.release_at_v);
        let delay = por.release_delay_ns;
        if rearm {
            if self.por_latched {
                self.enqueue_reset(PowerResetCause::PowerOn)?;
                self.restore_brown_out_defaults();
                self.por_event_sent = true;
            }
            self.por_latched = false;
            self.por_since_ns = None;
            self.bod_held = false;
            self.bod_active_at_ns = None;
            return Ok(());
        }
        if self.por_latched {
            return Ok(());
        }
        if !qualifies {
            self.por_since_ns = None;
            return Ok(());
        }
        let start = *self.por_since_ns.get_or_insert(time_ns);
        if time_ns - start >= delay {
            self.por_latched = true;
            self.por_since_ns = None;
            if !self.por_event_sent {
                self.enqueue_reset(PowerResetCause::PowerOn)?;
            }
            self.por_event_sent = false;
            self.restore_brown_out_defaults();
            self.bod_active_at_ns = self
                .config
                .brown_out
                .as_ref()
                .map(|bod| Self::deadline(time_ns, bod.startup_delay_ns))
                .transpose()?;
        }
        Ok(())
    }
    fn advance_brown_out(&mut self, time_ns: u64) -> Result<(), String> {
        let (Some(bod), Some(applied)) = (&self.config.brown_out, self.bod_config) else {
            return Ok(());
        };
        if !self.por_latched || !applied.enabled || !self.available(&bod.detector_domain) {
            self.bod_assert_since_ns = None;
            self.bod_release_since_ns = None;
            return Ok(());
        }
        let Some(volts) = self.voltage(&bod.sense_domain) else {
            self.bod_assert_since_ns = None;
            self.bod_release_since_ns = None;
            return Ok(());
        };
        let (assert_delay, release_delay, startup_delay) = (
            bod.assert_delay_ns,
            bod.release_delay_ns,
            bod.startup_delay_ns,
        );
        if self.bod_held {
            if volts >= applied.rising_v {
                let since = *self.bod_release_since_ns.get_or_insert(time_ns);
                if time_ns - since >= release_delay {
                    self.bod_held = false;
                    self.bod_release_since_ns = None;
                }
            } else {
                self.bod_release_since_ns = None;
            }
            return Ok(());
        }
        if self.bod_active_at_ns.is_none_or(|active| time_ns < active) {
            return Ok(());
        }
        if volts < applied.falling_v {
            let since = *self.bod_assert_since_ns.get_or_insert(time_ns);
            if time_ns - since >= assert_delay {
                self.bod_held = true;
                self.enqueue_reset(PowerResetCause::BrownOut)?;
                self.restore_brown_out_defaults();
                self.bod_active_at_ns = Some(Self::deadline(time_ns, startup_delay)?);
            }
        } else {
            self.bod_assert_since_ns = None;
        }
        Ok(())
    }
}
