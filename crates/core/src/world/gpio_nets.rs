// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The `gpio_net` half of a [`World`](super::World): joining pads of
//! different nodes into nets, and moving edges between them.
//!
//! The method:
//!
//! 1. A node runs. A node that has a net delivery due on the way stops at the
//!    delivery's cycle and drives the pad through `set_gpio_input` (so EXTI
//!    and timer captures see a real edge), then runs on. The cycle is the
//!    first instruction boundary at or after `t_edge + latency`, which is the
//!    same boundary however the run is cut into pieces.
//! 2. After it ran, the node's pad drive changes (four-state ring, stamped
//!    with engine cycles) are read, put on world time, and merged into their
//!    net in time order, but only up to the time every member node has
//!    reached: a member that is still behind may yet report an earlier
//!    change. The merge resolves the wire and queues a delivery for
//!    `t_edge + latency` (see [`GpioNet`]).
//! 3. A node never runs past its *safe horizon*: for each net it is on, the
//!    time the slowest member of that net has reached plus the net's latency.
//!    An edge no node has reported yet lies after the reporting node's time,
//!    so its delivery lies after the horizon: every delivery a node needs
//!    is known before the node gets there, whatever order the nodes run in
//!    and however far apart their clocks are.
//!
//! [`WorldGpio::run_to`] does this with one clock per node (conservative
//! parallel discrete-event simulation): a node on no net, or only on slow
//! nets, is not held to the shortest latency, and the bookkeeping between two
//! runs of a node is a few array reads. [`World::step_all`](super::World::step_all)
//! drives it one round at a time; the older lockstep driver
//! ([`WorldGpio::advance_node`] + [`WorldGpio::merge_edges`]) remains for
//! worlds that also have a timed UART network, and as the reference the tests
//! compare against.
//!
//! Pads whose GPIO model pushes its edges (`GpioPort` families and the
//! ATmega `avr_gpio` port) are read from the push tap at full speed. A model
//! without push capture would be sampled by the machine's per-cycle poll:
//! exact, but the machine then runs one instruction at a time and does not
//! fast-forward idle time.

use super::{MachineTrait, UART_NET_STEP_CYCLES};
use crate::logic_capture::PadState;
use crate::network::gpio_net::{GpioNet, Own};
use crate::network::timed_uart::{cycles_to_ps, ps_to_cycles_ceil};
use crate::SimResult;
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
    id: String,
    hz: u64,
    /// Channels below this are marker channels (they share the watch set).
    marker_count: u32,
    /// Cursor into the node's four-state ring.
    state_cursor: u64,
    /// Cursor into the level ring, acknowledged here when no marker consumer
    /// reads it (otherwise the timed-UART marker pass does).
    level_cursor: u64,
    bindings: Vec<NetBinding>,
    /// The nets this node is on, each once, ascending.
    nets: Vec<usize>,
    /// The earliest delivery this node must apply, on its own cycle axis;
    /// `None` inside means none is queued. Cleared (outer `None`) whenever a
    /// net of this node queues or hands over a delivery.
    due: Option<Option<Due>>,
}

/// One machine of the world as the per-node scheduler sees it.
struct Slot {
    id: String,
    hz: u64,
    /// Index into [`WorldGpio::nodes`] when the machine has net pads.
    node: Option<usize>,
    /// World time the machine has reached (its cycle count on world time).
    t_ps: u64,
    /// Made no progress when asked to run in the current call (halted,
    /// locked up): it neither holds others back nor runs again this call.
    stalled: bool,
    failed: bool,
    /// This call: the cycle count it started at, the cycle the target time
    /// is at, the cycle the call may take it to, and the first error.
    start: u64,
    round_goal: u64,
    goal: u64,
    error: Option<crate::SimulationError>,
}

/// What one [`WorldGpio::run_to`] call did.
pub(super) struct RunOutcome {
    /// Per machine id: the first error its run returned, else `Ok`.
    pub(super) results: HashMap<String, SimResult<()>>,
    /// Every machine with a clock reached the target or made no progress.
    pub(super) all_there: bool,
}

/// The GPIO nets of a world and the drive changes not yet merged.
pub(super) struct WorldGpio {
    pub(super) nets: Vec<GpioNet>,
    /// Net nodes, sorted by id.
    nodes: Vec<NodeGpio>,
    /// Per net: the indices into `nodes` of its member nodes, each once.
    net_nodes: Vec<Vec<usize>>,
    /// Per net, per member: drive changes `(world ps, drive)` seen but not
    /// yet merged into the net, in time order.
    pending: Vec<Vec<VecDeque<(u64, Own)>>>,
    /// Per net: how many changes `pending` holds.
    pending_len: Vec<usize>,
    /// One round's length: at most the shortest latency of any net, since no
    /// edge can reach a peer in less. Defaults to exactly that.
    pub(super) round_ps: u64,
    /// Every delivery applied to a pad (capped), by due time, then node id.
    applied: Vec<AppliedDelivery>,
    /// Every machine of the world in id order, for [`Self::run_to`]. Rebuilt
    /// when the world's set of machines changes.
    slots: Vec<Slot>,
    /// Per net node: its index into `slots`.
    node_slot: Vec<usize>,
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
#[derive(Debug, Clone, Copy)]
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
                    id: node.clone(),
                    hz,
                    marker_count,
                    state_cursor: 0,
                    level_cursor: 0,
                    bindings: Vec::new(),
                    nets: Vec::new(),
                    due: None,
                });
                entry.bindings.push(NetBinding {
                    net: ni,
                    member: mi,
                    ch: marker_count + ch_in_net_pins,
                });
                if !entry.nets.contains(&ni) {
                    entry.nets.push(ni);
                }
            }
            nets.push(GpioNet::new(label, cfg.pull, latency_ps, members));
        }
        // Watch: markers (probes on the pad), then net pads (each chip's own
        // output stage, `PinPort::driver`). Join the net pads first so their
        // peripheral lines know they share a wire before the watch arms.
        let mut watch_nodes: BTreeSet<&String> = net_pins.keys().collect();
        watch_nodes.extend(marker_pins.keys());
        for node in watch_nodes {
            let machine = machines.get_mut(node).expect("validated");
            let markers: Vec<(String, u8)> = marker_pins.get(node).cloned().unwrap_or_default();
            let pads: &[(String, u8)] = net_pins.get(node).map_or(&[], Vec::as_slice);
            if !pads.is_empty() {
                machine
                    .join_net_pads(pads)
                    .with_context(|| format!("gpio_net on node '{node}'"))?;
            }
            machine
                .watch_world_pins(&markers, pads)
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
        // BTreeMap order: `nodes` is sorted by id.
        let nodes: Vec<NodeGpio> = nodes.into_values().collect();
        let net_nodes = (0..nets.len())
            .map(|ni| {
                (0..nodes.len())
                    .filter(|&gi| nodes[gi].nets.contains(&ni))
                    .collect()
            })
            .collect();
        Ok(Some(Self {
            pending_len: vec![0; nets.len()],
            nets,
            nodes,
            net_nodes,
            pending,
            round_ps,
            applied: Vec::new(),
            slots: Vec::new(),
            node_slot: Vec::new(),
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

    fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.binary_search_by(|g| g.id.as_str().cmp(id)).ok()
    }

    /// The earliest delivery net node `gi` must apply, on its own cycle axis
    /// (cached until one of its nets queues or hands over a delivery).
    fn earliest_due(&mut self, gi: usize) -> Option<Due> {
        if let Some(due) = self.nodes[gi].due {
            return due;
        }
        let g = &self.nodes[gi];
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
        self.nodes[gi].due = Some(best);
        best
    }

    /// Every member node of net `ni` must look for its next delivery again.
    fn forget_dues(&mut self, ni: usize) {
        for &gi in &self.net_nodes[ni] {
            self.nodes[gi].due = None;
        }
    }

    /// Advance node `id` toward `target_cycle` (at most one step budget),
    /// applying every net delivery that falls due on the way at its cycle.
    /// The lockstep driver's half of a round; see [`Self::run_to`] for the
    /// per-node one.
    pub(super) fn advance_node(
        &mut self,
        id: &str,
        machine: &mut dyn MachineTrait,
        target_cycle: u64,
    ) -> SimResult<()> {
        let budget_end = target_cycle.min(machine.total_cycles() + UART_NET_STEP_CYCLES);
        match self.node_index(id) {
            Some(gi) => self.run_node(gi, machine, budget_end),
            None => {
                if machine.total_cycles() < budget_end {
                    machine.advance_to_cycle(budget_end)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Run net node `gi` to `limit`, applying each delivery at its cycle.
    fn run_node(&mut self, gi: usize, machine: &mut dyn MachineTrait, limit: u64) -> SimResult<()> {
        loop {
            let now = machine.total_cycles();
            while let Some(due) = self.earliest_due(gi) {
                if due.cycle > now {
                    break;
                }
                self.apply_due(gi, machine, due, now);
            }
            if now >= limit {
                return Ok(());
            }
            let mut stop = limit;
            if let Some(due) = self.earliest_due(gi) {
                if due.cycle < stop {
                    stop = due.cycle.max(now + 1);
                }
            }
            machine.advance_to_cycle(stop)?;
            if machine.total_cycles() == now {
                return Ok(());
            }
        }
    }

    /// Drive the pad of `due` on node `gi` (at cycle `now`) and log it.
    fn apply_due(&mut self, gi: usize, machine: &mut dyn MachineTrait, due: Due, now: u64) {
        let b = due.binding;
        let net = &self.nets[b.net];
        let m = &net.members()[b.member];
        machine.drive_gpio_input(&m.peripheral, m.pin, due.level);
        if self.applied.len() < MAX_APPLIED_LOG {
            let entry = AppliedDelivery {
                node: self.nodes[gi].id.clone(),
                peripheral: m.peripheral.clone(),
                pin: m.pin,
                cycle: now,
                due_ps: net.next_due(b.member).map_or(0, |d| d.t_ps),
                level: due.level,
            };
            // Nodes run on their own clocks, so which of two nodes applies
            // the same edge first is a scheduling detail: keep the log in
            // (due time, node id) order, each node's own entries in the order
            // it applied them. Entries arrive nearly sorted.
            let mut at = self.applied.len();
            while at > 0 && {
                let prev = &self.applied[at - 1];
                (prev.due_ps, prev.node.as_str()) > (entry.due_ps, entry.node.as_str())
            } {
                at -= 1;
            }
            self.applied.insert(at, entry);
        }
        self.nets[b.net].consume(b.member);
        self.forget_dues(b.net);
    }

    /// Move node `gi`'s new pad drive changes into `pending`.
    fn collect_edges(&mut self, gi: usize, machine: &mut dyn MachineTrait) {
        let g = &mut self.nodes[gi];
        let (edges, next) = machine.net_pad_states(g.state_cursor);
        g.state_cursor = next;
        for (ch, cycle, state) in edges {
            if let Some(b) = g.bindings.iter().find(|b| b.ch == ch) {
                self.pending[b.net][b.member]
                    .push_back((cycles_to_ps(cycle, g.hz), own_of(Some(state))));
                self.pending_len[b.net] += 1;
            }
        }
        if g.marker_count == 0 {
            // Nobody else reads the level ring: acknowledge it.
            let (_, next) = machine.marker_edges(g.level_cursor);
            g.level_cursor = next;
        }
    }

    /// Merge into net `ni` its pending changes at or before `horizon`, in
    /// time order (ties: member order), queueing the deliveries.
    fn merge_net(&mut self, ni: usize, horizon: u64) {
        let mut merged = false;
        loop {
            // The earliest unmerged change across members.
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
            self.pending_len[ni] -= changes.len();
            self.nets[ni].apply(t, &changes);
            merged = true;
        }
        if merged {
            self.forget_dues(ni);
        }
    }

    /// Read every net node's new pad drive changes and merge into the nets all
    /// those at or before the time every `reached` node has got to. The
    /// lockstep driver's end of a round.
    pub(super) fn merge_edges(
        &mut self,
        machines: &mut HashMap<String, Box<dyn MachineTrait>>,
        reached: &BTreeSet<String>,
    ) {
        for gi in 0..self.nodes.len() {
            let Some(machine) = machines.get_mut(&self.nodes[gi].id) else {
                continue;
            };
            self.collect_edges(gi, machine.as_mut());
        }
        for ni in 0..self.nets.len() {
            // Everything at or before the time all member nodes have reached
            // is known; a node that has not reached the round end made no
            // progress and cannot report anything new.
            let horizon = self.nets[ni]
                .members()
                .iter()
                .filter(|m| reached.contains(&m.node))
                .filter_map(|m| {
                    machines
                        .get(&m.node)
                        .map(|mc| cycles_to_ps(mc.total_cycles(), m.hz))
                })
                .min();
            if let Some(horizon) = horizon {
                self.merge_net(ni, horizon);
            }
        }
    }

    /// Make `slots` describe the world's machines.
    fn rebuild_slots(
        &mut self,
        machines: &HashMap<String, Box<dyn MachineTrait>>,
        node_hz: &HashMap<String, u64>,
    ) {
        let mut ids: Vec<&String> = machines.keys().collect();
        ids.sort();
        self.slots = ids
            .into_iter()
            .map(|id| Slot {
                id: id.clone(),
                hz: node_hz.get(id).copied().unwrap_or(0),
                node: self.node_index(id),
                t_ps: 0,
                stalled: false,
                failed: false,
                start: 0,
                round_goal: 0,
                goal: 0,
                error: None,
            })
            .collect();
        self.node_slot = self
            .nodes
            .iter()
            .map(|g| {
                self.slots
                    .iter()
                    .position(|s| s.id == g.id)
                    .expect("every net node is a machine of the world")
            })
            .collect();
    }

    /// The latest world time net node `gi` may run to: per net it is on, the
    /// time the slowest live member has reached plus the net's latency.
    /// Includes the node itself, whose own edges come back to it too.
    fn horizon_ps(&self, gi: usize) -> u64 {
        let mut h = u64::MAX;
        for &ni in &self.nodes[gi].nets {
            let slowest = self.net_nodes[ni]
                .iter()
                .map(|&m| &self.slots[self.node_slot[m]])
                .filter(|s| !s.stalled)
                .map(|s| s.t_ps)
                .min();
            if let Some(t) = slowest {
                h = h.min(t.saturating_add(self.nets[ni].latency_ps));
            }
        }
        h
    }

    /// Merge each net of node `gi` up to the time its live members reached.
    fn merge_nets_of(&mut self, gi: usize) {
        for k in 0..self.nodes[gi].nets.len() {
            let ni = self.nodes[gi].nets[k];
            if self.pending_len[ni] == 0 {
                continue;
            }
            let horizon = self.net_nodes[ni]
                .iter()
                .map(|&m| &self.slots[self.node_slot[m]])
                .filter(|s| !s.stalled)
                .map(|s| s.t_ps)
                .min();
            if let Some(h) = horizon {
                self.merge_net(ni, h);
            }
        }
    }

    /// Run every machine of the world to world time `until_ps` (each to the
    /// first instruction boundary at or after it), never letting a net node
    /// pass its safe horizon, and at most `step_cycles` cycles per machine
    /// when given. Machines with no known clock do not run. Nodes are tried
    /// in id order, each as far as it may go, until none can move.
    pub(super) fn run_to(
        &mut self,
        machines: &mut HashMap<String, Box<dyn MachineTrait>>,
        node_hz: &HashMap<String, u64>,
        until_ps: u64,
        step_cycles: Option<u64>,
    ) -> RunOutcome {
        // The machines in slot order, looked up once per call. A world
        // whose set of machines changed since the last call gets new slots.
        if self.slots.len() != machines.len()
            || machines.keys().any(|id| {
                self.slots
                    .binary_search_by(|s| s.id.as_str().cmp(id))
                    .is_err()
            })
        {
            self.rebuild_slots(machines, node_hz);
        }
        let n = self.slots.len();
        let mut ms: Vec<Option<&mut Box<dyn MachineTrait>>> = Vec::with_capacity(n);
        ms.resize_with(n, || None);
        for (id, m) in machines.iter_mut() {
            if let Ok(i) = self.slots.binary_search_by(|s| s.id.as_str().cmp(id)) {
                ms[i] = Some(m);
            }
        }
        for (s, m) in self.slots.iter_mut().zip(&ms) {
            let now = m.as_ref().map_or(0, |m| m.total_cycles());
            s.start = now;
            s.t_ps = cycles_to_ps(now, s.hz);
            s.stalled = false;
            s.failed = false;
            s.error = None;
            if s.hz != 0 {
                s.round_goal = ps_to_cycles_ceil(until_ps, s.hz);
                s.goal =
                    step_cycles.map_or(s.round_goal, |c| s.round_goal.min(now.saturating_add(c)));
            }
        }
        loop {
            // A node that ran, or that turned out stalled, moves horizons.
            let mut changed = false;
            for (i, entry) in ms.iter_mut().enumerate() {
                let Some(machine) = entry.as_mut() else {
                    continue;
                };
                let machine: &mut dyn MachineTrait = &mut ***machine;
                let (hz, node, stalled, failed) = {
                    let s = &self.slots[i];
                    (s.hz, s.node, s.stalled, s.failed)
                };
                if hz == 0 || stalled || failed {
                    continue;
                }
                let now = machine.total_cycles();
                let mut limit = self.slots[i].goal;
                if let Some(gi) = node {
                    let h = self.horizon_ps(gi);
                    if h != u64::MAX {
                        limit = limit.min(ps_to_cycles_ceil(h, hz));
                    }
                }
                if limit <= now {
                    continue;
                }
                let r = match node {
                    Some(gi) => self.run_node(gi, machine, limit),
                    None => machine.advance_to_cycle(limit),
                };
                let after = machine.total_cycles();
                let s = &mut self.slots[i];
                s.t_ps = cycles_to_ps(after, hz);
                // A machine that made no progress (halted, locked up) cannot
                // hold the others back.
                s.stalled = after == now;
                changed = true;
                if let Err(e) = r {
                    s.failed = true;
                    s.error = Some(e);
                }
                if let Some(gi) = node {
                    self.collect_edges(gi, machine);
                    self.merge_nets_of(gi);
                }
            }
            if !changed {
                break;
            }
        }
        let mut all_there = true;
        let mut results = HashMap::with_capacity(n);
        for (s, m) in self.slots.iter_mut().zip(&ms) {
            let after = m.as_ref().map_or(0, |m| m.total_cycles());
            if s.hz != 0 && after < s.round_goal && after > s.start {
                all_there = false;
            }
            results.insert(s.id.clone(), s.error.take().map_or(Ok(()), Err));
        }
        RunOutcome { results, all_there }
    }
}
