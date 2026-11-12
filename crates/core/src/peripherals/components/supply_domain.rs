//! Functional external supply routing, fed by existing plant voltages.
//! Legacy single domains keep their minimum-only gate. Declared device rails
//! use inclusive bounded ranges and one atomic AND gate per device. This
//! models operating availability, not silicon damage, inrush or rail loading.
use super::supply::powered_from_config;
use crate::{
    inspect::{Artifact, InspectOpts},
    peripherals::{device::I2cDevice, kit::AttachCtx},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

mod spi;
pub use spi::GatedSpiDevice;

pub const MAX_DEVICE_SUPPLY_RAILS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclaredRailBounds {
    min_v: f64,
    max_v: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
struct RailBounds {
    min_v: f64,
    max_v: Option<f64>,
}
impl RailBounds {
    fn contains(self, voltage: f64) -> bool {
        voltage >= self.min_v && self.max_v.is_none_or(|max| voltage <= max)
    }
}
#[derive(Debug)]
pub struct SupplyDomain {
    rails: BTreeMap<String, RailBounds>,
    state: Mutex<DomainState>,
}
#[derive(Debug, Default, Clone)]
struct DomainState {
    voltages: BTreeMap<String, f64>,
    on: bool,
    generation: u64,
}
impl SupplyDomain {
    fn new(rails: BTreeMap<String, RailBounds>) -> Self {
        Self {
            rails,
            state: Mutex::new(DomainState::default()),
        }
    }
    pub fn powered(&self) -> bool {
        self.state.lock().unwrap().on
    }
    pub fn generation(&self) -> u64 {
        self.state.lock().unwrap().generation
    }
    fn next_state(
        &self,
        old: &DomainState,
        updates: &BTreeMap<String, f64>,
    ) -> anyhow::Result<DomainState> {
        let mut next = old.clone();
        for (rail, voltage) in updates {
            anyhow::ensure!(
                self.rails.contains_key(rail),
                "unknown supply rail '{rail}'"
            );
            anyhow::ensure!(
                voltage.is_finite() && *voltage >= 0.0,
                "supply voltage must be finite and nonnegative"
            );
            next.voltages.insert(rail.clone(), *voltage);
        }
        let on = self.rails.iter().all(|(rail, bounds)| {
            next.voltages
                .get(rail)
                .is_some_and(|voltage| bounds.contains(*voltage))
        });
        if on != old.on {
            next.generation = old
                .generation
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("supply generation exhausted"))?;
        }
        next.on = on;
        Ok(next)
    }
    fn update(&self, voltage: f64) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap();
        *state = self.next_state(&state, &BTreeMap::from([(String::new(), voltage)]))?;
        Ok(())
    }
    fn rail_evidence(&self) -> serde_json::Value {
        let state = self.state.lock().unwrap();
        serde_json::Value::Object(
            self.rails
                .iter()
                .map(|(rail, bounds)| {
                    let voltage = state.voltages.get(rail).copied();
                    let status = match voltage {
                        None => "unknown",
                        Some(v) if bounds.contains(v) => "in_range",
                        Some(_) => "out_of_range",
                    };
                    (
                        rail.clone(),
                        serde_json::json!({
                            "min_v": bounds.min_v, "max_v": bounds.max_v,
                            "voltage_v": voltage, "state": status
                        }),
                    )
                })
                .collect(),
        )
    }
}

#[derive(Clone)]
pub struct SupplyGate {
    domain: Option<Arc<SupplyDomain>>,
    enabled: bool,
}
impl SupplyGate {
    pub fn routed(domain: Arc<SupplyDomain>, enabled: bool) -> Self {
        Self {
            domain: Some(domain),
            enabled,
        }
    }
    pub fn legacy(enabled: bool) -> Self {
        Self {
            domain: None,
            enabled,
        }
    }
    pub fn powered(&self) -> bool {
        self.enabled && self.domain.as_ref().is_none_or(|d| d.powered())
    }
    pub fn generation(&self) -> Option<u64> {
        self.domain.as_ref().map(|d| d.generation())
    }
    pub fn is_routed(&self) -> bool {
        self.domain.is_some()
    }
    pub fn validate_required_rails(
        &self,
        required: &[crate::peripherals::device::DeviceSupplyRail],
    ) -> anyhow::Result<()> {
        let Some(domain) = &self.domain else {
            return Ok(());
        };
        anyhow::ensure!(
            !required.is_empty(),
            "SPI device declares no supply rail operating limits"
        );
        anyhow::ensure!(
            domain.rails.len() == required.len(),
            "supply_rails must name every required device rail exactly once"
        );
        let names: std::collections::BTreeSet<_> = required.iter().map(|rail| rail.name).collect();
        anyhow::ensure!(
            names.len() == required.len() && names.iter().all(|name| valid_rail_name(name)),
            "model declares duplicate or invalid supply rail names"
        );
        for rail in required {
            anyhow::ensure!(
                rail.min_v.is_finite()
                    && rail.max_v.is_finite()
                    && rail.min_v > 0.0
                    && rail.min_v <= rail.max_v,
                "model declares invalid operating limits for '{}'",
                rail.name
            );
            let bounds = domain.rails.get(rail.name).ok_or_else(|| {
                anyhow::anyhow!("supply_rails is missing required rail '{}'", rail.name)
            })?;
            anyhow::ensure!(
                bounds.min_v >= rail.min_v && bounds.max_v.is_some_and(|max| max <= rail.max_v),
                "declared rail '{}' exceeds the model operating envelope {}..={} V",
                rail.name,
                rail.min_v,
                rail.max_v
            );
        }
        Ok(())
    }
    pub fn annotate(&self, mut artifact: Artifact) -> Artifact {
        if let Some(meta) = artifact.meta.as_object_mut() {
            meta.insert("powered".into(), self.powered().into());
            meta.insert(
                "supply_generation".into(),
                serde_json::json!(self.generation()),
            );
            if let Some(domain) = &self.domain {
                meta.insert("supply_rails".into(), domain.rail_evidence());
                meta.insert(
                    "supply_scope".into(),
                    "Operating availability only; no damage, inrush or load-current model.".into(),
                );
            }
        }
        artifact
    }
    pub fn from_config(ctx: &mut AttachCtx<'_>) -> anyhow::Result<Self> {
        let enabled = powered_from_config(ctx);
        if let Some(value) = ctx.ext.config.get("supply_rails") {
            anyhow::ensure!(
                !ctx.ext.config.contains_key("supply_domain")
                    && !ctx.ext.config.contains_key("supply_min_v"),
                "supply_rails cannot be combined with supply_domain/supply_min_v"
            );
            if let Some(powered) = ctx.ext.config.get("powered") {
                anyhow::ensure!(
                    powered.as_bool().is_some(),
                    "routed powered must be a boolean"
                );
            }
            let rails: BTreeMap<String, DeclaredRailBounds> = serde_yaml::from_value(value.clone())
                .map_err(|error| anyhow::anyhow!("invalid supply_rails: {error}"))?;
            anyhow::ensure!(
                (1..=MAX_DEVICE_SUPPLY_RAILS).contains(&rails.len()),
                "supply_rails must declare 1..={MAX_DEVICE_SUPPLY_RAILS} rails"
            );
            let mut bounds = BTreeMap::new();
            for (rail, limits) in rails {
                anyhow::ensure!(valid_rail_name(&rail), "invalid supply rail name '{rail}'");
                anyhow::ensure!(
                    limits.min_v.is_finite()
                        && limits.max_v.is_finite()
                        && limits.min_v > 0.0
                        && limits.min_v <= limits.max_v,
                    "supply rail '{rail}' requires finite 0 < min_v <= max_v"
                );
                bounds.insert(
                    rail,
                    RailBounds {
                        min_v: limits.min_v,
                        max_v: Some(limits.max_v),
                    },
                );
            }
            anyhow::ensure!(
                !ctx.ext.id.is_empty(),
                "routed device requires a nonempty id"
            );
            anyhow::ensure!(
                !ctx.bus.supply_domains.devices.contains_key(&ctx.ext.id),
                "device '{}' already declares supply rails",
                ctx.ext.id
            );
            let domain = Arc::new(SupplyDomain::new(bounds));
            ctx.bus
                .supply_domains
                .devices
                .insert(ctx.ext.id.clone(), domain.clone());
            return Ok(Self::routed(domain, enabled));
        }
        let Some(value) = ctx.ext.config.get("supply_domain") else {
            anyhow::ensure!(
                !ctx.ext.config.contains_key("supply_min_v"),
                "supply_min_v requires supply_domain"
            );
            return Ok(Self::legacy(enabled));
        };
        let name = value
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("invalid supply_domain"))?;
        let domain = ctx
            .bus
            .supply_domains
            .legacy
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown supply domain '{name}'"))?;
        let min = ctx
            .config_f64("supply_min_v")
            .ok_or_else(|| anyhow::anyhow!("routed device requires supply_min_v"))?;
        anyhow::ensure!(
            min.is_finite()
                && min > 0.0
                && domain
                    .rails
                    .get("")
                    .is_some_and(|bounds| bounds.min_v == min),
            "incompatible functional supply threshold"
        );
        Ok(Self::routed(domain.clone(), enabled))
    }
}
fn valid_rail_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Default)]
pub(crate) struct SupplyDomains {
    legacy: HashMap<String, Arc<SupplyDomain>>,
    devices: HashMap<String, Arc<SupplyDomain>>,
}
impl crate::bus::SystemBus {
    pub fn register_supply_domain(
        &mut self,
        name: &str,
        min_v: f64,
    ) -> anyhow::Result<Arc<SupplyDomain>> {
        anyhow::ensure!(
            !name.is_empty() && min_v.is_finite() && min_v > 0.0,
            "invalid functional supply domain/threshold"
        );
        if let Some(domain) = self.supply_domains.legacy.get(name) {
            anyhow::ensure!(
                domain.rails[""].min_v == min_v,
                "incompatible existing supply threshold"
            );
            return Ok(domain.clone());
        }
        let domain = Arc::new(SupplyDomain::new(BTreeMap::from([(
            String::new(),
            RailBounds { min_v, max_v: None },
        )])));
        self.supply_domains
            .legacy
            .insert(name.into(), domain.clone());
        Ok(domain)
    }
    pub fn update_supply_domain(&self, name: &str, voltage_v: f64) -> anyhow::Result<()> {
        self.supply_domains
            .legacy
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown supply domain '{name}'"))?
            .update(voltage_v)
    }
    pub fn validate_device_supply_route(&self, device: &str, rail: &str) -> anyhow::Result<()> {
        let domain = self
            .supply_domains
            .devices
            .get(device)
            .ok_or_else(|| anyhow::anyhow!("device '{device}' declares no supply_rails"))?;
        anyhow::ensure!(
            domain.rails.contains_key(rail),
            "device '{device}' has no supply rail '{rail}'"
        );
        Ok(())
    }
    /// Validate the complete boundary before mutation. Publish each device's
    /// rails together, so a rail swap cannot invent an intermediate power edge.
    pub fn apply_device_supply_voltages(
        &mut self,
        updates: &[(String, String, f64)],
    ) -> anyhow::Result<()> {
        let mut grouped: BTreeMap<&str, BTreeMap<String, f64>> = BTreeMap::new();
        for (device, rail, voltage) in updates {
            self.validate_device_supply_route(device, rail)?;
            anyhow::ensure!(
                voltage.is_finite() && *voltage >= 0.0,
                "device '{device}' rail '{rail}' voltage must be finite and nonnegative"
            );
            anyhow::ensure!(
                grouped
                    .entry(device)
                    .or_default()
                    .insert(rail.clone(), *voltage)
                    .is_none(),
                "duplicate device supply update '{device}.{rail}'"
            );
        }
        let mut prepared = Vec::with_capacity(grouped.len());
        for (device, rails) in grouped {
            let domain = self.supply_domains.devices[device].clone();
            let state = domain.next_state(&domain.state.lock().unwrap(), &rails)?;
            prepared.push((domain, state));
        }
        if prepared.is_empty() {
            return Ok(());
        }
        for (domain, state) in prepared {
            *domain.state.lock().unwrap() = state;
        }
        // Publish through the existing mutable SPI-device capability before
        // inspection or CPU execution. No elapsed time or bus frame is added.
        for peripheral in &mut self.peripherals {
            if let Some(devices) = peripheral.dev.spi_attached_devices_mut() {
                for device in devices {
                    device.sync_supply_state();
                }
            }
        }
        self.service_device_pin_drives();
        Ok(())
    }
}
pub struct GatedI2cDevice {
    inner: Box<dyn I2cDevice>,
    gate: SupplyGate,
}
impl GatedI2cDevice {
    pub fn new(inner: Box<dyn I2cDevice>, gate: SupplyGate) -> Self {
        Self { inner, gate }
    }
}
impl I2cDevice for GatedI2cDevice {
    fn address(&self) -> u8 {
        self.inner.address()
    }
    fn transaction_generation(&self) -> Option<u64> {
        self.gate
            .generation()
            .or_else(|| self.inner.transaction_generation())
    }
    fn claims_address(&self, a: u8) -> bool {
        self.gate.powered() && self.inner.claims_address(a)
    }
    fn clock_stretch_ns(&self) -> u64 {
        if self.gate.powered() {
            self.inner.clock_stretch_ns()
        } else {
            0
        }
    }
    fn select_address(&mut self, a: u8) {
        if self.gate.powered() {
            self.inner.select_address(a)
        }
    }
    fn read(&mut self) -> u8 {
        if self.gate.powered() {
            self.inner.read()
        } else {
            0xff
        }
    }
    fn write(&mut self, b: u8) {
        if self.gate.powered() {
            self.inner.write(b)
        }
    }
    fn start(&mut self) {
        if self.gate.powered() {
            self.inner.start()
        }
    }
    fn stop(&mut self) {
        if self.gate.powered() {
            self.inner.stop()
        }
    }
    fn advance_time_us(&mut self, us: u64) {
        if self.gate.powered() {
            self.inner.advance_time_us(us)
        }
    }
    fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        if self.gate.powered() {
            self.inner.take_pin_drives()
        } else {
            vec![]
        }
    }
    fn artifacts(&self, id: &str, opts: &InspectOpts) -> Vec<Artifact> {
        self.inner
            .artifacts(id, opts)
            .into_iter()
            .map(|artifact| self.gate.annotate(artifact))
            .collect()
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        self.inner.logs()
    }
    fn wrapped_device(&self) -> Option<&dyn I2cDevice> {
        Some(self.inner.as_ref())
    }
    fn wrapped_device_mut(&mut self) -> Option<&mut dyn crate::peripherals::device::I2cDevice> {
        Some(self.inner.as_mut())
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        self.inner.as_sim_input_mut()
    }
    fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        self.inner.for_each_sim_input(f)
    }
}
