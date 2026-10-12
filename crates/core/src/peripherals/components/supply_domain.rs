//! Functional external supply routing, fed by an existing plant's voltage.
//! No battery model or silicon thresholds. Routed domains start off. I2C
//! register state is retained on cutoff; only bus access and clocks are gated.
use super::supply::powered_from_config;
use crate::{
    inspect::{Artifact, InspectOpts},
    peripherals::{device::I2cDevice, kit::AttachCtx},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
#[derive(Debug)]
pub struct SupplyDomain {
    min_v: f64,
    state: Mutex<DomainState>,
}
#[derive(Debug, Default)]
struct DomainState {
    voltage_v: Option<f64>,
    on: bool,
    generation: u64,
}
impl SupplyDomain {
    pub fn powered(&self) -> bool {
        self.state.lock().unwrap().on
    }
    pub fn generation(&self) -> u64 {
        self.state.lock().unwrap().generation
    }
    fn update(&self, v: f64) -> anyhow::Result<()> {
        anyhow::ensure!(
            v.is_finite() && v >= 0.0,
            "supply voltage must be finite and nonnegative"
        );
        let mut s = self.state.lock().unwrap();
        let on = v >= self.min_v;
        if on != s.on {
            s.generation = s
                .generation
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("supply generation exhausted"))?;
            s.on = on;
        }
        s.voltage_v = Some(v);
        Ok(())
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
    pub fn from_config(ctx: &AttachCtx<'_>) -> anyhow::Result<Self> {
        let enabled = powered_from_config(ctx);
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
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown supply domain '{name}'"))?;
        let min = ctx
            .config_f64("supply_min_v")
            .ok_or_else(|| anyhow::anyhow!("routed device requires supply_min_v"))?;
        anyhow::ensure!(
            min.is_finite() && min > 0.0 && min == domain.min_v,
            "incompatible functional supply threshold"
        );
        Ok(Self::routed(domain.clone(), enabled))
    }
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
        if let Some(d) = self.supply_domains.get(name) {
            anyhow::ensure!(d.min_v == min_v, "incompatible existing supply threshold");
            return Ok(d.clone());
        }
        let d = Arc::new(SupplyDomain {
            min_v,
            state: Mutex::new(DomainState::default()),
        });
        self.supply_domains.insert(name.into(), d.clone());
        Ok(d)
    }
    pub fn update_supply_domain(&self, name: &str, voltage_v: f64) -> anyhow::Result<()> {
        self.supply_domains
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown supply domain '{name}'"))?
            .update(voltage_v)
    }
}
pub(crate) type SupplyDomains = HashMap<String, Arc<SupplyDomain>>;
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
            .map(|mut a| {
                if let Some(m) = a.meta.as_object_mut() {
                    m.insert("powered".into(), self.gate.powered().into());
                    m.insert(
                        "supply_generation".into(),
                        serde_json::json!(self.gate.generation()),
                    );
                }
                a
            })
            .collect()
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        self.inner.logs()
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.inner.as_any()
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        self.inner.as_any_mut()
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
