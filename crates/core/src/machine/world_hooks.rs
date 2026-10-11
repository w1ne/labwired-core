// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! What a multi-node [`World`](crate::world::World) needs from one machine to
//! run a timed network: advance to a cycle, wake its timed USARTs, reset the
//! node, and report GPIO marker edges.

use crate::logic_capture::{
    LogicEdge, LogicEdgeBatch, LogicSource, LogicStateBatch, LogicStateEdge, PadState,
};
use crate::{AdvanceRequest, AdvanceStop, Cpu, Machine, SimResult};
use std::collections::VecDeque;

/// Most instrument edges held between two reads, per ring.
const OBSERVER_CAP: usize = 65_536;

/// One channel an instrument wants on a node: a pad, or a peripheral's own wire.
#[derive(Debug, Clone)]
pub struct ObserverRef {
    /// `"gpio"` or `"wire"`.
    pub kind: String,
    pub peripheral: String,
    /// Pin, for a `gpio` ref.
    pub pin: u8,
    /// Line name, for a `wire` ref.
    pub line: Option<String>,
}

/// What arming one instrument channel came to.
#[derive(Debug, Clone, Default)]
pub struct ObserverRow {
    /// The level now, `None` when unknown.
    pub initial: Option<bool>,
    /// Why the ref did not resolve, when it did not.
    pub error: Option<String>,
}

/// Lets a world and an instrument watch the same machine.
///
/// A machine has ONE logic ring, and reading it acknowledges what it passes, so
/// a world that reads its marker and GPIO-net pads and an instrument that
/// reads its own channels would eat each other's edges. Once an instrument
/// arms, every read of the rings goes through [`Machine::observer_pull`], which
/// files each edge under its owner by channel: channels below `limit` are the
/// world's (markers, then net pads), the rest are the instrument's.
#[derive(Default)]
pub(crate) struct ObserverMux {
    /// The world's own watch (markers, then net pads), as last installed.
    world_sources: Vec<Option<LogicSource>>,
    /// Set by the first instrument arm; the world's cursors are ignored from
    /// then on, because arming restarts the rings.
    engaged: bool,
    level_pulled: u64,
    state_pulled: u64,
    world_levels: Vec<(u32, u64, bool)>,
    world_states: Vec<(u32, u64, PadState)>,
    ui_edges: VecDeque<(u64, LogicEdge)>,
    ui_states: VecDeque<(u64, LogicStateEdge)>,
    ui_edge_seq: u64,
    ui_state_seq: u64,
    ui_dropped: u64,
    ui_initial: Vec<Option<PadState>>,
    ui_sources: Vec<Option<LogicSource>>,
}

impl<C: Cpu> Machine<C> {
    /// Run until `total_cycles >= target` (or the CPU stops making progress).
    /// Batched, with idle fast-forward, as a continuous run.
    pub fn advance_to_cycle(&mut self, target: u64) -> SimResult<()> {
        let mut guard = 0u32;
        while self.total_cycles < target {
            let report = self
                .advance(AdvanceRequest::run(None).with_cycle_limit(target - self.total_cycles))?;
            if matches!(
                report.stop,
                AdvanceStop::NoProgress
                    | AdvanceStop::FirmwareExit { .. }
                    | AdvanceStop::NetDriveChange
            ) {
                break;
            }
            guard += 1;
            if guard > 1_000_000 {
                break;
            }
        }
        Ok(())
    }

    /// Like [`Self::advance_to_cycle`], but stop at the first boundary at
    /// which one of this machine's watched pads records a four-state change
    /// (a world's `gpio_net` pads, see [`Self::watch_world_pins`]): `true`
    /// when it stopped for one. A push already queued (a pad driven while
    /// paused) ends the run after one instruction, when it is recorded.
    ///
    /// Exact only where every CPU batch ends at a batch-stop push
    /// ([`crate::logic_capture::LogicTap::set_batch_stop_channels`]): one core.
    /// See [`Self::net_drive_stop_exact`].
    pub fn advance_to_cycle_or_net_drive_change(&mut self, target: u64) -> SimResult<bool> {
        if self.total_cycles >= target {
            return Ok(false);
        }
        let from = self.logic_capture.state_seq();
        self.net_drive_stop_from = from;
        // A queued push is recorded only after the next instruction; a fast
        // path that does not look at the tap (the AVR INC/RJMP spin) must not
        // run past it.
        let first = if self.bus.logic_tap.batch_stop_hit() {
            target.min(self.total_cycles + 1)
        } else {
            target
        };
        let r = self.advance_to_cycle(first).and_then(|()| {
            if first < target && self.logic_capture.state_seq() == from {
                self.advance_to_cycle(target)
            } else {
                Ok(())
            }
        });
        self.net_drive_stop_from = u64::MAX;
        self.net_drive_stop_hit = false;
        r.map(|()| self.logic_capture.state_seq() > from)
    }

    /// Whether [`Self::advance_to_cycle_or_net_drive_change`] stops exactly at
    /// the boundary a net pad change is recorded at: push capture only (a
    /// polled pad is sampled at the boundary anyway, but this keeps the
    /// promise simple) and one core (a dual-core window steps its parked
    /// core between peripheral ticks without looking at the tap).
    pub fn net_drive_stop_exact(&self) -> bool {
        self.cpu_secondary.is_none() && !self.logic_capture.poll_active()
    }

    /// The cycle before which this machine changes no pad unless an input
    /// does: `Some` only while the core sleeps (`WFI`, `SLEEP`) with idle
    /// fast-forward on and would skip straight to its next scheduled event,
    /// which is the cycle returned. Every condition under which
    /// `try_idle_fast_forward` would not skip, or would stop early, answers
    /// `None`.
    pub fn idle_quiet_until(&self) -> Option<u64> {
        // The core's own answer first: a busy core is the common case.
        let budget = self
            .config
            .idle_fast_forward_enabled
            .then(|| self.cpu.idle_fast_forward_budget(&self.bus))
            .flatten()?;
        // Only an event-scheduler machine fast-forwards, and only one whose
        // scheduler has started (`drain_scheduler_events` sets the flag, and
        // only in that build).
        if budget == 0
            || !self.scheduler_bootstrapped
            || self.logic_capture.poll_active()
            || self.cpu_secondary.is_some()
            || self.bus.requires_cycle_accurate()
            || self.bus.supply.is_routed()
            || self.bus.idle_poll_bus_tick_active()
            || self.bus.logic_tap.pending_len() != 0
            || !self.bus.idle_fast_forward_legacy_safe()
        {
            return None;
        }
        let now = self.total_cycles;
        let mut wake = now.saturating_add(budget);
        for deadline in [
            self.sched.next_event_deadline(),
            self.bus.pending_schedule.iter().map(|(_, d, _)| *d).min(),
            self.bus.next_motor_service_deadline_cycle(),
            self.bus.next_resident_edge_deadline_cycle(),
        ]
        .into_iter()
        .flatten()
        {
            if deadline <= now {
                return None;
            }
            wake = wake.min(deadline);
        }
        Some(wake)
    }

    /// Hand every timed USART a chance to schedule a wake for a character a
    /// peer put on the wire since it last looked. A world calls this at the
    /// start of each synchronisation round.
    pub fn timed_uart_sync(&mut self) {
        // Only an event-scheduler run drains `sched`; on the per-cycle walk a
        // timed USART services itself every tick and needs no wake.
        if !self.scheduler_bootstrapped {
            return;
        }
        let now = self.total_cycles;
        for idx in 0..self.bus.peripherals.len() {
            if let Some(target) = self.bus.peripherals[idx].dev.claim_timed_uart_wake(now) {
                self.sched.schedule(target, idx as u32, 0);
            }
        }
    }

    /// Reset the node the way its reset pin does: the core restarts through
    /// its reset vector, the NVIC and SysTick return to their reset state, and
    /// every peripheral gets [`crate::Peripheral::on_node_reset`]. SRAM keeps
    /// its contents, as on silicon.
    pub fn reset_node(&mut self) -> SimResult<()> {
        if let Some(nvic) = &self.bus.nvic {
            use std::sync::atomic::Ordering;
            for w in 0..8 {
                nvic.iser[w].store(0, Ordering::SeqCst);
                nvic.ispr[w].store(0, Ordering::SeqCst);
                nvic.iabr[w].store(0, Ordering::SeqCst);
                nvic.level_pended[w].store(0, Ordering::SeqCst);
            }
            for p in nvic.ipr.iter() {
                p.store(0, Ordering::SeqCst);
            }
        }
        for p in self.bus.peripherals.iter_mut() {
            p.dev.on_node_reset();
        }
        self.reset()
    }

    /// Watch GPIO pads for marker edges: `(gpio peripheral id, pin)` each.
    pub fn watch_marker_pins(&mut self, pins: &[(String, u8)]) -> anyhow::Result<()> {
        self.watch_world_pins(pins, &[])
    }

    /// Watch a world's pads: `markers` as probes on the pad (channels
    /// `0..markers.len()`), then the `gpio_net` members `net_pads` as their
    /// chip's own output stage ([`LogicSource::Driver`]): what this chip
    /// drives, never the level the net holds on the pad. Call
    /// [`Self::join_net_pads`] for the net pads first.
    pub fn watch_world_pins(
        &mut self,
        markers: &[(String, u8)],
        net_pads: &[(String, u8)],
    ) -> anyhow::Result<()> {
        let mut sources = Vec::with_capacity(markers.len() + net_pads.len());
        for (k, (name, pin)) in markers.iter().chain(net_pads).enumerate() {
            let idx = self
                .bus
                .find_peripheral_index_by_name(name)
                .ok_or_else(|| anyhow::anyhow!("no peripheral '{name}'"))?;
            sources.push(Some(if k < markers.len() {
                LogicSource::pad(idx, *pin)
            } else {
                LogicSource::driver(idx, *pin)
            }));
        }
        self.observer.world_sources = sources.clone();
        self.logic_watch(&sources);
        // A push on a net pad ends the CPU batch it happens in, so a run to
        // the next drive change stops on that instruction.
        let lo = markers.len() as u32;
        self.bus
            .logic_tap
            .set_batch_stop_channels(lo, lo + net_pads.len() as u32);
        Ok(())
    }

    /// Marker edges since `cursor`: `(channel, cycle, level)`, and the cursor
    /// to pass next time.
    pub fn marker_edges(&mut self, cursor: u64) -> (Vec<(u32, u64, bool)>, u64) {
        if self.observer.engaged {
            self.observer_pull();
            return (std::mem::take(&mut self.observer.world_levels), cursor);
        }
        let batch = self.logic_read_edges(cursor);
        (
            batch
                .edges
                .iter()
                .map(|e| (e.ch, e.cycle, e.value))
                .collect(),
            batch.cursor,
        )
    }

    /// Put GPIO pads `(peripheral id, pin)` on a world `gpio_net`: each pad's
    /// port is told the pad now shares a wire with other chips
    /// ([`PinPort::join_wire`](crate::pins::PinPort::join_wire)). Nothing about
    /// the pad's drive changes: the net reads each member's own output stage
    /// ([`PinPort::driver`](crate::pins::PinPort::driver)), which never
    /// included the level the net feeds back. Refuses a pad whose peripheral
    /// owns no pins, or whose drive is not known (a pad routed to a
    /// peripheral signal the model does not publish), naming it.
    pub fn join_net_pads(&mut self, pins: &[(String, u8)]) -> anyhow::Result<()> {
        // A net pad routed to a peripheral line (SPI, I²C) hands the levels
        // the net delivers to that peripheral.
        self.bus.wire_inputs_live = true;
        for (name, pin) in pins {
            let idx = self
                .bus
                .find_peripheral_index_by_name(name)
                .ok_or_else(|| anyhow::anyhow!("no peripheral '{name}'"))?;
            let Some(port) = self.bus.peripherals[idx].dev.pins_mut() else {
                anyhow::bail!(
                    "peripheral '{name}' cannot take part in a GPIO net (pin {pin}): it owns no pins"
                );
            };
            if *pin >= port.pin_count() {
                anyhow::bail!(
                    "peripheral '{name}' cannot take part in a GPIO net (pin {pin}): its port has {} pins",
                    port.pin_count()
                );
            }
            if port.driver(*pin).is_none() {
                anyhow::bail!(
                    "pad {name}.{pin} cannot be on a GPIO net: its drive is not known (is it routed to a peripheral signal the model does not publish?)"
                );
            }
            port.join_wire(*pin);
        }
        Ok(())
    }

    /// Four-state drive changes of the watched channels since `cursor`:
    /// `(channel, cycle, state)`, and the cursor to pass next time.
    pub fn net_pad_states(
        &mut self,
        cursor: u64,
    ) -> (Vec<(u32, u64, crate::logic_capture::PadState)>, u64) {
        if self.observer.engaged {
            self.observer_pull();
            return (std::mem::take(&mut self.observer.world_states), cursor);
        }
        let batch = self.logic_read_states(cursor);
        (
            batch
                .edges
                .iter()
                .map(|e| (e.ch, e.cycle, e.state))
                .collect(),
            batch.cursor,
        )
    }

    /// Four-state value of each watched channel when the watch was armed.
    pub fn net_pad_initial_states(&self) -> Vec<Option<crate::logic_capture::PadState>> {
        self.logic_initial_states().to_vec()
    }

    /// Hold `pin` of GPIO peripheral `name` at `level` as an external driver
    /// would, through the same seam a board button uses (EXTI edges and timer
    /// captures fire). `false` if the pad does not resolve or cannot be driven.
    pub fn drive_gpio_input(&mut self, name: &str, pin: u8, level: bool) -> bool {
        match self.bus.find_peripheral_index_by_name(name) {
            Some(idx) => self.bus.set_peripheral_gpio_input(idx, pin, level),
            None => false,
        }
    }

    /// Move everything the rings hold into the world's and the instrument's
    /// buffers, so neither read hides edges from the other.
    fn observer_pull(&mut self) {
        let limit = self.observer.world_sources.len() as u32;
        let levels = self.logic_read_edges(self.observer.level_pulled);
        self.observer.level_pulled = levels.cursor;
        self.observer.ui_dropped = levels.dropped;
        for e in levels.edges {
            if e.ch < limit {
                self.observer.world_levels.push((e.ch, e.cycle, e.value));
            } else {
                self.observer.ui_edge_seq += 1;
                let seq = self.observer.ui_edge_seq;
                self.observer.ui_edges.push_back((
                    seq - 1,
                    LogicEdge {
                        ch: e.ch - limit,
                        ..e
                    },
                ));
                if self.observer.ui_edges.len() > OBSERVER_CAP {
                    self.observer.ui_edges.pop_front();
                }
            }
        }
        let states = self.logic_read_states(self.observer.state_pulled);
        self.observer.state_pulled = states.cursor;
        for e in states.edges {
            if e.ch < limit {
                self.observer.world_states.push((e.ch, e.cycle, e.state));
            } else {
                self.observer.ui_state_seq += 1;
                let seq = self.observer.ui_state_seq;
                self.observer.ui_states.push_back((
                    seq - 1,
                    LogicStateEdge {
                        ch: e.ch - limit,
                        ..e
                    },
                ));
                if self.observer.ui_states.len() > OBSERVER_CAP {
                    self.observer.ui_states.pop_front();
                }
            }
        }
    }

    /// Arm an instrument's watch set on this node, next to whatever the world
    /// watches. Replaces the previous instrument set; an empty slice disarms.
    /// Channels are numbered from 0 within `refs`.
    pub fn observer_watch(&mut self, refs: &[ObserverRef]) -> Vec<ObserverRow> {
        // What the world already put in the rings must not be lost to the
        // restart an arm causes.
        if self.observer.engaged || !self.observer.world_sources.is_empty() {
            if !self.observer.engaged {
                // First arm: the world's edges so far are in the rings and its
                // cursors are about to go stale.
                self.observer.engaged = true;
            }
            self.observer_pull();
        }
        self.observer.engaged = true;
        let mut rows = vec![ObserverRow::default(); refs.len()];
        let mut resolved: Vec<Option<LogicSource>> = Vec::with_capacity(refs.len());
        for (i, r) in refs.iter().enumerate() {
            let source = match r.kind.as_str() {
                "gpio" => match self.bus.find_peripheral_index_by_name(&r.peripheral) {
                    Some(idx) => Some(LogicSource::pad(idx, r.pin)),
                    None => {
                        rows[i].error = Some(
                            crate::logic_capture::LogicRefError::UnknownPeripheral {
                                peripheral: r.peripheral.clone(),
                            }
                            .to_string(),
                        );
                        None
                    }
                },
                "wire" => {
                    match self.resolve_wire_source(&r.peripheral, r.line.as_deref().unwrap_or("")) {
                        Ok(source) => Some(source),
                        Err(e) => {
                            rows[i].error = Some(e.to_string());
                            None
                        }
                    }
                }
                other => {
                    rows[i].error = Some(
                        crate::logic_capture::LogicRefError::UnknownKind {
                            kind: other.to_string(),
                        }
                        .to_string(),
                    );
                    None
                }
            };
            resolved.push(source);
        }
        let limit = self.observer.world_sources.len();
        let mut combined = self.observer.world_sources.clone();
        combined.extend(resolved.iter().copied());
        let initial = self.logic_watch(&combined);
        self.observer.level_pulled = 0;
        self.observer.state_pulled = 0;
        self.observer.ui_edges.clear();
        self.observer.ui_states.clear();
        self.observer.ui_edge_seq = 0;
        self.observer.ui_state_seq = 0;
        self.observer.ui_dropped = 0;
        self.observer.ui_initial = self
            .logic_initial_states()
            .iter()
            .skip(limit)
            .copied()
            .collect();
        self.observer.ui_sources = resolved;
        for (i, v) in initial.iter().skip(limit).enumerate() {
            rows[i].initial = *v;
        }
        rows
    }

    /// The instrument's level edges since `cursor` (see
    /// [`Machine::logic_read_edges`] for the cursor contract).
    pub fn observer_edges(&mut self, cursor: u64) -> LogicEdgeBatch {
        self.observer_pull();
        while self
            .observer
            .ui_edges
            .front()
            .is_some_and(|(s, _)| *s < cursor)
        {
            self.observer.ui_edges.pop_front();
        }
        LogicEdgeBatch {
            cursor: self.observer.ui_edge_seq,
            dropped: self.observer.ui_dropped,
            edges: self.observer.ui_edges.iter().map(|(_, e)| *e).collect(),
        }
    }

    /// The instrument's four-state edges since `cursor`, with each channel's
    /// state at arm time.
    pub fn observer_states(&mut self, cursor: u64) -> (LogicStateBatch, Vec<Option<PadState>>) {
        self.observer_pull();
        while self
            .observer
            .ui_states
            .front()
            .is_some_and(|(s, _)| *s < cursor)
        {
            self.observer.ui_states.pop_front();
        }
        (
            LogicStateBatch {
                cursor: self.observer.ui_state_seq,
                dropped: self.observer.ui_dropped,
                edges: self.observer.ui_states.iter().map(|(_, e)| *e).collect(),
            },
            self.observer.ui_initial.clone(),
        )
    }

    /// The level each of `refs` reads now, without arming anything.
    pub fn observer_sample(&self, refs: &[ObserverRef]) -> Vec<Option<bool>> {
        refs.iter()
            .map(|r| match r.kind.as_str() {
                "gpio" => self
                    .bus
                    .find_peripheral_index_by_name(&r.peripheral)
                    .and_then(|idx| self.bus.peripherals[idx].dev.read_gpio_pad(r.pin)),
                "wire" => self
                    .resolve_wire_source(&r.peripheral, r.line.as_deref().unwrap_or(""))
                    .ok()
                    .and_then(|s| self.read_logic_level(s)),
                _ => None,
            })
            .collect()
    }

    /// A GPIO pin's output latch (`output`) or input level, for board LEDs and
    /// buttons.
    pub fn gpio_level(&self, name: &str, pin: u8, output: bool) -> Option<bool> {
        let idx = self.bus.find_peripheral_index_by_name(name)?;
        let dev = &self.bus.peripherals[idx].dev;
        if output {
            dev.read_gpio_output(pin)
        } else {
            dev.read_gpio_input(pin)
        }
    }
}
