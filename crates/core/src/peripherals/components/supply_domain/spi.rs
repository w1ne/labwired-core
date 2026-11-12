//! A routed SPI supply gate with an explicit model power-reset capability.
use super::SupplyGate;
use crate::{
    inspect::{Artifact, InspectOpts},
    peripherals::device::{SpiDevice, SpiSampling},
};

pub struct GatedSpiDevice {
    inner: Box<dyn SpiDevice>,
    gate: SupplyGate,
    generation: Option<u64>,
    powered: bool,
    pending_pins: Vec<(String, bool)>,
}
impl GatedSpiDevice {
    pub fn new(mut inner: Box<dyn SpiDevice>, gate: SupplyGate) -> anyhow::Result<Self> {
        anyhow::ensure!(
            inner.supports_power_control(),
            "SPI device does not support routed power transitions"
        );
        gate.validate_required_rails(inner.required_supply_rails())?;
        let powered = gate.powered();
        inner.power_state_changed(powered);
        Ok(Self {
            generation: gate.generation(),
            inner,
            gate,
            powered,
            pending_pins: Vec::new(),
        })
    }
    fn sync_power(&mut self) {
        let generation = self.gate.generation();
        let powered = self.gate.powered();
        if generation == self.generation && powered == self.powered {
            return;
        }
        // A cut and restore between controller polls still resets the part.
        if self.powered && (!powered || generation != self.generation) {
            self.inner.take_pin_drives(); // discard unsent pre-cut active edges
            self.inner.power_state_changed(false);
            self.pending_pins.extend(self.inner.take_pin_drives());
        }
        if powered && (!self.powered || generation != self.generation) {
            self.inner.power_state_changed(true);
        }
        self.generation = generation;
        self.powered = powered;
    }
}
impl SpiDevice for GatedSpiDevice {
    fn sync_supply_state(&mut self) {
        self.sync_power();
    }
    fn sampling(&self) -> SpiSampling {
        self.inner.sampling()
    }
    fn needs_external_bus_poll(&self) -> bool {
        self.inner.needs_external_bus_poll()
    }
    fn component_id(&self) -> Option<&str> {
        self.inner.component_id()
    }
    fn attach_can_bus(
        &mut self,
        tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
        rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        self.inner.attach_can_bus(tx, rx)
    }
    fn poll_external_bus(&mut self) {
        self.sync_power();
        if self.powered {
            self.inner.poll_external_bus();
        }
    }
    fn cs_select(&mut self) {
        self.sync_power();
        if self.powered {
            self.inner.cs_select();
        }
    }
    fn cs_release(&mut self) {
        self.sync_power();
        if self.powered {
            self.inner.cs_release();
        }
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        self.sync_power();
        if self.powered {
            self.inner.transfer(byte)
        } else {
            0xff
        }
    }
    fn cs_pin(&self) -> &str {
        self.inner.cs_pin()
    }
    fn dc_pin(&self) -> Option<&str> {
        self.inner.dc_pin()
    }
    fn set_dc_level(&mut self, level: bool) {
        self.sync_power();
        if self.powered {
            self.inner.set_dc_level(level);
        }
    }
    fn dc_source(&self) -> Option<(u64, u8)> {
        self.inner.dc_source()
    }
    fn set_dc_source(&mut self, address: u64, bit: u8) {
        self.inner.set_dc_source(address, bit);
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
    fn advance_time_us(&mut self, us: u64) {
        self.sync_power();
        if self.powered && us != 0 {
            self.inner.advance_time_us(us);
        }
    }
    fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        self.sync_power();
        let mut pins = std::mem::take(&mut self.pending_pins);
        if self.powered {
            pins.extend(self.inner.take_pin_drives());
        }
        pins
    }
    fn artifacts(&self, id: &str, opts: &InspectOpts) -> Vec<Artifact> {
        self.inner
            .artifacts(id, opts)
            .into_iter()
            .map(|artifact| self.gate.annotate(artifact))
            .collect()
    }
    fn restore_runtime_snapshot(&mut self, _bytes: &[u8]) -> crate::SimResult<()> {
        Err(crate::SimulationError::NotImplemented(
            "routed SPI power-domain snapshots".into(),
        ))
    }
}
