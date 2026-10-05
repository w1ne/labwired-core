// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The `gpio_net` half of a [`World`](super::World): joining pads of
//! different nodes into nets, and moving edges between them.
//!
//! The method, in the order a round runs it:
//!
//! 1. Every node runs a round. A node that has a net delivery due inside the
//!    round stops at the delivery's cycle and drives the pad through
//!    `set_gpio_input` (so EXTI and timer captures see a real edge), then runs
//!    on. The cycle is the first instruction boundary at or after
//!    `t_edge + latency`, which is the same boundary whatever the round size.
//! 2. When the round is complete, each node's pad drive changes (four-state
//!    ring, stamped with engine cycles) are read, put on world time, and
//!    merged into their net in time order, but only up to the time every node
//!    has reached: a node that is still behind may yet report an earlier
//!    change. The merge resolves the wire and queues a delivery for
//!    `t_edge + latency` (see [`GpioNet`]).
//! 3. A round is never longer than the shortest net latency, so a delivery
//!    queued in step 2 is always still ahead of every node.
//!
//! Pads whose GPIO model pushes its edges (STM32 and the other `GpioPort`
//! families) are read from the push tap at full speed. A model without push
//! capture (the ATmega port until it gets one) is sampled by the machine's
//! per-cycle poll: exact, but the machine then runs one instruction at a time
//! and does not fast-forward idle time.

use super::{MachineTrait, UART_NET_STEP_CYCLES};
use crate::logic_capture::PadState;
use crate::network::gpio_net::{GpioNet, Own};
use crate::network::timed_uart::{cycles_to_ps, ps_to_cycles_ceil};
use anyhow::Context;
use labwired_config::GpioNetConfig;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

/// One pad of a node that sits on a net: which net and member, and the
/// watch channel that reports its drive.
#[derive(Debug, Clone, Copy)]
struct NetBinding {
    net: usize,
    member: usize,
    ch: u32,
}

/// Per node: the net pads it owns and where its drive-change cursors are.
struct NodeGpio {
    hz: u64,
    /// Channels below this are marker channels (they share the watch set).
    marker_count: u32,
    /// Cursor into the node's four-state ring.
    state_cursor: u64,
    /// Cursor into the level ring, acknowledged here when no marker consumer
    /// reads it (otherwise the timed-UART marker pass does).
    level_cursor: u64,
    bindings: Vec<NetBinding>,
}

/// The GPIO nets of a world and the drive changes not yet merged.
pub(super) struct WorldGpio {
    pub(super) nets: Vec<GpioNet>,
    nodes: BTreeMap<String, NodeGpio>,
    /// Per net, per member: drive changes `(world ps, drive)` seen but not
    /// yet merged into the net, in time order.
    pending: Vec<Vec<VecDeque<(u64, Own)>>>,
    /// One round's length: at most the shortest latency of any net, since no
    /// edge can reach a peer in less. Defaults to exactly that.
    pub(super) round_ps: u64,
    /// Every delivery applied to a pad, in the order applied (capped).
    applied: Vec<AppliedDelivery>,
}

/// Most applied deliveries kept in the log.
const MAX_APPLIED_LOG: usize = 65_536;

/// One net level change applied to a member pad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedDelivery {
    pub node: String,
    pub peripheral: String,
    pub pin: u8,
    /// The node's cycle when the level was applied.
    pub cycle: u64,
    /// World time the wire carried the change to this pad, ps.
    pub due_ps: u64,
    pub level: bool,
}

fn own_of(state: Option<PadState>) -> Own {
    match state {
        Some(PadState::Low) => Own::Low,
        Some(PadState::High) => Own::High,
        Some(PadState::WeakHigh) => Own::PullUp,
        Some(PadState::WeakLow) => Own::PullDown,
        _ => Own::Z,
    }
}

/// A delivery a node must apply: its cycle and where it goes.
struct Due {
    cycle: u64,
    binding: NetBinding,
    level: bool,
}

impl WorldGpio {
    /// Build the nets of a world, watch the pads (markers first, then net
    /// pads) and put every net at its initial level.
    pub(super) fn build(
        machines: &mut HashMap<String, Box<dyn MachineTrait>>,
        node_hz: &HashMap<String, u64>,
        marker_pins: &BTreeMap<String, Vec<(String, u8)>>,
        configs: &[GpioNetConfig],
    ) -> anyhow::Result<Option<Self>> {
        if configs.is_empty() {
            return Ok(None);
        }
        // Net pads per node, in net then member order.
        let mut nets = Vec::with_capacity(configs.len());
        let mut nodes: BTreeMap<String, NodeGpio> = BTreeMap::new();
        let mut net_pins: BTreeMap<String, Vec<(String, u8)>> = BTreeMap::new();
        let mut taken: BTreeMap<(String, String, u8), usize> = BTreeMap::new();
        for (ni, cfg) in configs.iter().enumerate() {
            let label = cfg.name.clone().unwrap_or_else(|| format!("net{ni}"));
            let mut members = Vec::new();
            for m in &cfg.members {
                if !machines.contains_key(&m.node) {
                    anyhow::bail!("gpio_net '{label}': unknown node '{}'", m.node);
                }
                let hz = node_hz.get(&m.node).copied().unwrap_or(0);
                if hz == 0 {
                    anyhow::bail!(
                        "gpio_net '{label}': node '{}' has no known CPU clock; nets step by time",
                        m.node
                    );
                }
                if let Some(other) = taken.insert((m.node.clone(), m.peripheral.clone(), m.pin), ni)
                {
                    anyhow::bail!(
                        "pad {}.{}.{} is on two nets (net {other} and '{label}'); merge them into one",
                        m.node,
                        m.peripheral,
                        m.pin
                    );
                }
                members.push((m.node.clone(), m.peripheral.clone(), m.pin, hz));
            }
            let latency_ps = cfg.latency_ns.saturating_mul(1000);
            // Floor: the latency cannot be shorter than one cycle of the
            // slowest member, which the round cannot resolve.
            for (node, _, _, hz) in &members {
                let period_ps = 1_000_000_000_000u64.div_ceil(*hz);
                if latency_ps < period_ps {
                    anyhow::bail!(
                        "gpio_net '{label}': latency_ns {} is below one cycle of node '{node}' ({:.1} ns); \
                         the floor is one round lookahead",
                        cfg.latency_ns,
                        period_ps as f64 / 1000.0
                    );
                }
            }
            for (mi, (node, periph, pin, _)) in members.iter().enumerate() {
                let pins = net_pins.entry(node.clone()).or_default();
                let ch_in_net_pins = pins.len() as u32;
                pins.push((periph.clone(), *pin));
                let marker_count = marker_pins.get(node).map_or(0, Vec::len) as u32;
                let hz = node_hz[node];
                let entry = nodes.entry(node.clone()).or_insert_with(|| NodeGpio {
                    hz,
                    marker_count,
                    state_cursor: 0,
                    level_cursor: 0,
                    bindings: Vec::new(),
                });
                entry.bindings.push(NetBinding {
                    net: ni,
                    member: mi,
                    ch: marker_count + ch_in_net_pins,
                });
            }
            nets.push(GpioNet::new(label, cfg.pull, latency_ps, members));
        }
        // Watch: markers, then net pads. Isolate the net pads first so the
        // watch seeds their own drive.
        let mut watch_nodes: BTreeSet<&String> = net_pins.keys().collect();
        watch_nodes.extend(marker_pins.keys());
        for node in watch_nodes {
            let machine = machines.get_mut(node).expect("validated");
            let mut all: Vec<(String, u8)> = marker_pins.get(node).cloned().unwrap_or_default();
            if let Some(pins) = net_pins.get(node) {
                machine
                    .isolate_net_pads(pins)
                    .with_context(|| format!("gpio_net on node '{node}'"))?;
                all.extend(pins.iter().cloned());
            }
            machine
                .watch_marker_pins(&all)
                .with_context(|| format!("pad watch on node '{node}'"))?;
        }
        // Initial drives, initial levels.
        let initial: BTreeMap<String, Vec<Option<PadState>>> = nodes
            .keys()
            .map(|n| (n.clone(), machines[n].net_pad_initial_states()))
            .collect();
        for (ni, net) in nets.iter_mut().enumerate() {
            let own: Vec<Own> = (0..net.members().len())
                .map(|mi| {
                    let (node, b) = nodes
                        .iter()
                        .find_map(|(n, g)| {
                            g.bindings
                                .iter()
                                .find(|b| b.net == ni && b.member == mi)
                                .map(|b| (n, *b))
                        })
                        .expect("every member has a binding");
                    own_of(initial[node].get(b.ch as usize).copied().flatten())
                })
                .collect();
            let level = net.init(&own);
            for m in net.members().to_vec() {
                let machine = machines.get_mut(&m.node).expect("validated");
                if !machine.drive_gpio_input(&m.peripheral, m.pin, level) {
                    anyhow::bail!(
                        "gpio_net '{}': pad {}.{}.{} cannot be driven from outside \
                         (its GPIO model has no external input)",
                        net.name,
                        m.node,
                        m.peripheral,
                        m.pin
                    );
                }
            }
        }
        let round_ps = nets.iter().map(|n| n.latency_ps).min().unwrap_or(u64::MAX);
        let pending = nets
            .iter()
            .map(|n| vec![VecDeque::new(); n.members().len()])
            .collect();
        Ok(Some(Self {
            nets,
            nodes,
            pending,
            round_ps,
            applied: Vec::new(),
        }))
    }

    pub(super) fn applied(&self) -> &[AppliedDelivery] {
        &self.applied
    }

    /// Shorten rounds below the default (the shortest net latency). Rounds
    /// can be anything up to that; results must not depend on it, which is
    /// what the determinism tests vary.
    pub(super) fn set_round_ps(&mut self, round_ps: u64) -> anyhow::Result<()> {
        let max = self
            .nets
            .iter()
            .map(|n| n.latency_ps)
            .min()
            .unwrap_or(u64::MAX);
        if round_ps == 0 || round_ps > max {
            anyhow::bail!("a round must be between 1 ps and the shortest net latency ({max} ps)");
        }
        self.round_ps = round_ps;
        Ok(())
    }

    /// The earliest delivery node `id` must apply, on its own cycle axis.
    fn earliest_due(&self, id: &str) -> Option<Due> {
        let g = self.nodes.get(id)?;
        let mut best: Option<Due> = None;
        for b in &g.bindings {
            let Some(d) = self.nets[b.net].next_due(b.member) else {
                continue;
            };
            let cycle = ps_to_cycles_ceil(d.t_ps, g.hz);
            if best.as_ref().is_none_or(|x| cycle < x.cycle) {
                best = Some(Due {
                    cycle,
                    binding: *b,
                    level: d.level,
                });
            }
        }
        best
    }

    /// Advance node `id` toward `target_cycle` (at most one step budget),
    /// applying every net delivery that falls due on the way at its cycle.
    pub(super) fn advance_node(
        &mut self,
        id: &str,
        machine: &mut dyn MachineTrait,
        target_cycle: u64,
    ) -> crate::SimResult<()> {
        if !self.nodes.contains_key(id) {
            let before = machine.total_cycles();
            if before < target_cycle {
                return machine.advance_to_cycle(target_cycle.min(before + UART_NET_STEP_CYCLES));
            }
            return Ok(());
        }
        let budget_end = target_cycle.min(machine.total_cycles() + UART_NET_STEP_CYCLES);
        loop {
            let now = machine.total_cycles();
            while let Some(due) = self.earliest_due(id) {
                if due.cycle > now {
                    break;
                }
                let net = &self.nets[due.binding.net];
                let m = &net.members()[due.binding.member];
                let (periph, pin) = (m.peripheral.clone(), m.pin);
                machine.drive_gpio_input(&periph, pin, due.level);
                if self.applied.len() < MAX_APPLIED_LOG {
                    self.applied.push(AppliedDelivery {
                        node: id.to_string(),
                        peripheral: periph,
                        pin,
                        cycle: now,
                        due_ps: self.nets[due.binding.net]
                            .next_due(due.binding.member)
                            .map_or(0, |d| d.t_ps),
                        level: due.level,
                    });
                }
                self.nets[due.binding.net].consume(due.binding.member);
            }
            if now >= budget_end {
                return Ok(());
            }
            let mut limit = budget_end;
            if let Some(due) = self.earliest_due(id) {
                if due.cycle < limit {
                    limit = due.cycle.max(now + 1);
                }
            }
            machine.advance_to_cycle(limit)?;
            if machine.total_cycles() == now {
                return Ok(());
            }
        }
    }

    /// Read every net node's new pad drive changes and merge into the nets all
    /// those at or before the time every `reached` node has got to.
    pub(super) fn merge_edges(
        &mut self,
        machines: &mut HashMap<String, Box<dyn MachineTrait>>,
        reached: &BTreeSet<String>,
    ) {
        for (id, g) in self.nodes.iter_mut() {
            let Some(machine) = machines.get_mut(id) else {
                continue;
            };
            let (edges, next) = machine.net_pad_states(g.state_cursor);
            g.state_cursor = next;
            for (ch, cycle, state) in edges {
                if let Some(b) = g.bindings.iter().find(|b| b.ch == ch) {
                    self.pending[b.net][b.member]
                        .push_back((cycles_to_ps(cycle, g.hz), own_of(Some(state))));
                }
            }
            if g.marker_count == 0 {
                // Nobody else reads the level ring: acknowledge it.
                let (_, next) = machine.marker_edges(g.level_cursor);
                g.level_cursor = next;
            }
        }
        for (ni, net) in self.nets.iter_mut().enumerate() {
            // Everything at or before the time all member nodes have reached
            // is known; a node that has not reached the round end made no
            // progress and cannot report anything new.
            let horizon = net
                .members()
                .iter()
                .filter(|m| reached.contains(&m.node))
                .filter_map(|m| {
                    machines
                        .get(&m.node)
                        .map(|mc| cycles_to_ps(mc.total_cycles(), m.hz))
                })
                .min();
            let Some(horizon) = horizon else { continue };
            loop {
                // The earliest unmerged change across members (ties: member order).
                let t = self.pending[ni]
                    .iter()
                    .filter_map(|q| q.front().map(|(t, _)| *t))
                    .min();
                let Some(t) = t.filter(|t| *t <= horizon) else {
                    break;
                };
                let mut changes = Vec::new();
                for (mi, q) in self.pending[ni].iter_mut().enumerate() {
                    while q.front().is_some_and(|(qt, _)| *qt == t) {
                        let (_, own) = q.pop_front().expect("front checked");
                        changes.push((mi, own));
                    }
                }
                net.apply(t, &changes);
            }
        }
    }
}
