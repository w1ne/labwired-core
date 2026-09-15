// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Type erasure for [`crate::Machine`].
//!
//! `Machine<C: Cpu>` is generic over its CPU, so every caller that wants to
//! hold one either becomes generic itself or picks a concrete CPU. The CLI took
//! the first route and the browser the second, which is a large part of why
//! machine construction was copy-pasted per architecture.
//!
//! [`SessionMachine`] is the object-safe surface a session needs. It extends
//! [`DebugControl`] rather than restating it: breakpoints, registers,
//! `read_memory`, `inspect`, `peek`, `snapshot` and `restore` already live
//! there, and declaring them twice would make every call through a
//! `dyn SessionMachine` ambiguous. What is added here is the run loop
//! (`advance`), the clock (`cycles`), stimulus, logic capture, bus trace, the
//! bus accesses `DebugControl` has no shape for (a pin driven by name, a CAN
//! frame delivered to a named controller, a word read or written through the
//! bus's own width-aware path), and the two `Machine`
//! inherent methods whose names or shapes differ from the `DebugControl` ones
//! (`apply_snapshot`, `reset_machine`).

use crate::bus::bus_trace::BusTraceEvent;
use crate::logic_capture::{LogicEdgeBatch, LogicSource};
use crate::machine::{AdvanceReport, AdvanceRequest};
use crate::network::{CanFrame, CanInjectError};
use crate::sim_input::{InputChannel, SimInputError};
use crate::snapshot::MachineSnapshot;
use crate::{Bus, Cpu, DebugControl, Machine, SimResult};

/// Why [`SessionMachine::set_gpio_input`] did not drive a pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioInputError {
    /// No peripheral on the bus has that name.
    UnknownPeripheral,
    /// The peripheral exists but does not accept an externally driven level.
    NotDrivable,
}

/// Everything a session needs from a machine, without the `C: Cpu` generic.
pub trait SessionMachine: DebugControl + Send {
    /// Advance the machine through its authoritative execution path.
    fn advance(&mut self, request: AdvanceRequest) -> SimResult<AdvanceReport>;
    /// Simulated machine cycles since construction.
    fn cycles(&self) -> u64;
    /// Drive one simulated input channel.
    fn set_input(&mut self, channel: &str, value: f64) -> Result<(), SimInputError>;
    /// Apply several input sets as one atomic transaction.
    ///
    /// The pairs are `(channel, value)`; the component disambiguator the bus
    /// accepts is deliberately not exposed here, so a session caller addresses
    /// channels exactly as [`Self::set_input`] does.
    fn set_inputs(&mut self, sets: &[(String, f64)]) -> Result<(), SimInputError>;
    /// Enumerate the input channels the attached devices expose.
    fn list_inputs(&mut self) -> Vec<(String, InputChannel)>;
    /// Install a logic-analyzer watch set, returning each channel's level.
    fn logic_watch(&mut self, resolved: &[Option<LogicSource>]) -> Vec<Option<bool>>;
    /// Read logic edges newer than `cursor`.
    fn logic_read_edges(&mut self, cursor: u64) -> LogicEdgeBatch;
    /// The cycle the logic capture clock is at.
    fn logic_now_cycle(&self) -> u64;
    /// Restore a snapshot taken from this machine.
    fn apply_snapshot(&mut self, snap: &MachineSnapshot) -> SimResult<()>;
    /// Every retained bus-trace event, oldest first.
    fn bus_trace_events(&self) -> Vec<BusTraceEvent>;
    /// Reset the machine, as the reset line would.
    ///
    /// Named apart from [`DebugControl::reset`] so a call through a
    /// `dyn SessionMachine` is never ambiguous.
    fn reset_machine(&mut self) -> SimResult<()>;
    /// Hold `pin` of the named GPIO peripheral at `level`, as an external
    /// contact would: the same seam the browser's board-IO buttons use
    /// (`Peripheral::set_gpio_input`).
    fn set_gpio_input(
        &mut self,
        peripheral: &str,
        pin: u8,
        level: bool,
    ) -> Result<(), GpioInputError>;
    /// Read a 32-bit word through the bus's width-aware path (bit-band and
    /// atomic-alias decoding, peripheral word reads), which is what firmware
    /// and the CLI's `memory_value` assertion see. A four-byte
    /// [`DebugControl::read_memory`] can differ on those addresses.
    fn bus_read_u32(&self, addr: u64) -> SimResult<u32>;
    /// Write a 32-bit word through the bus's width-aware path, as firmware
    /// would.
    fn bus_write_u32(&mut self, addr: u64, value: u32) -> SimResult<()>;
    /// Deliver a frame to the named CAN controller's receive path
    /// ([`crate::bus::SystemBus::inject_can_frame`]).
    fn inject_can(&mut self, controller: &str, frame: CanFrame) -> Result<(), CanInjectError>;
}

impl<C: Cpu + 'static> SessionMachine for Machine<C> {
    fn advance(&mut self, request: AdvanceRequest) -> SimResult<AdvanceReport> {
        Machine::advance(self, request)
    }

    fn cycles(&self) -> u64 {
        self.total_cycles
    }

    fn set_input(&mut self, channel: &str, value: f64) -> Result<(), SimInputError> {
        Machine::set_input(self, channel, value)
    }

    fn set_inputs(&mut self, sets: &[(String, f64)]) -> Result<(), SimInputError> {
        // `Machine::set_inputs` takes `(component, channel, value)` triples so a
        // caller can disambiguate two devices exposing the same key. A session
        // addresses channels by key alone, so widen here rather than leaking
        // the selector into the trait.
        let triples: Vec<(Option<&str>, &str, f64)> = sets
            .iter()
            .map(|(channel, value)| (None, channel.as_str(), *value))
            .collect();
        Machine::set_inputs(self, &triples)
    }

    fn list_inputs(&mut self) -> Vec<(String, InputChannel)> {
        Machine::list_inputs(self)
    }

    fn logic_watch(&mut self, resolved: &[Option<LogicSource>]) -> Vec<Option<bool>> {
        Machine::logic_watch(self, resolved)
    }

    fn logic_read_edges(&mut self, cursor: u64) -> LogicEdgeBatch {
        Machine::logic_read_edges(self, cursor)
    }

    fn logic_now_cycle(&self) -> u64 {
        Machine::logic_now_cycle(self)
    }

    fn apply_snapshot(&mut self, snap: &MachineSnapshot) -> SimResult<()> {
        Machine::apply_snapshot(self, snap.clone())
    }

    fn bus_trace_events(&self) -> Vec<BusTraceEvent> {
        self.bus.bus_trace_snapshot()
    }

    fn reset_machine(&mut self) -> SimResult<()> {
        Machine::reset(self)
    }

    fn set_gpio_input(
        &mut self,
        peripheral: &str,
        pin: u8,
        level: bool,
    ) -> Result<(), GpioInputError> {
        let idx = self
            .bus
            .find_peripheral_index_by_name(peripheral)
            .ok_or(GpioInputError::UnknownPeripheral)?;
        if self.bus.peripherals[idx].dev.set_gpio_input(pin, level) {
            Ok(())
        } else {
            Err(GpioInputError::NotDrivable)
        }
    }

    fn bus_read_u32(&self, addr: u64) -> SimResult<u32> {
        Bus::read_u32(&self.bus, addr)
    }

    fn bus_write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        Bus::write_u32(&mut self.bus, addr, value)
    }

    fn inject_can(&mut self, controller: &str, frame: CanFrame) -> Result<(), CanInjectError> {
        self.bus.inject_can_frame(controller, frame)
    }
}
