// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::network::Interconnect;
use crate::{Bus, Cpu, Machine, SimResult};
use std::collections::HashMap;

mod gpio_nets;
use gpio_nets::WorldGpio;
pub use gpio_nets::{AppliedDelivery, GpioSchedulerStats};

/// Per-machine results of one [`World::step_all_into`] call, in machine id
/// order. Reused across calls: the ids are copied only when the world's set
/// of machines changes.
#[derive(Debug, Default)]
pub struct StepResults {
    ids: Vec<String>,
    results: Vec<SimResult<()>>,
}

impl StepResults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.results.len()
    }

    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// `(machine id, result)`, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &SimResult<()>)> {
        self.ids.iter().map(String::as_str).zip(&self.results)
    }

    /// The result of machine `id`, if it was stepped.
    pub fn get(&self, id: &str) -> Option<&SimResult<()>> {
        let i = self.ids.binary_search_by(|k| k.as_str().cmp(id)).ok()?;
        self.results.get(i)
    }

    /// The first machine, in id order, whose step returned an error.
    pub fn first_error(&self) -> Option<(&str, &crate::SimulationError)> {
        self.iter()
            .find_map(|(id, r)| r.as_ref().err().map(|e| (id, e)))
    }

    /// The same results as the map [`World::step_all`] returns.
    pub fn into_map(self) -> HashMap<String, SimResult<()>> {
        let mut map = HashMap::with_capacity(self.ids.len());
        for (id, r) in self.ids.into_iter().zip(self.results) {
            map.insert(id, r);
        }
        map
    }

    /// [`Self::into_map`] that keeps the ids for the next call.
    fn drain_to_map(&mut self) -> HashMap<String, SimResult<()>> {
        let mut map = HashMap::with_capacity(self.ids.len());
        for (id, r) in self.ids.iter().zip(self.results.drain(..)) {
            map.insert(id.clone(), r);
        }
        map
    }

    /// Start a call's results: these ids, in this (ascending) order.
    pub(crate) fn set_ids<'a>(&mut self, ids: impl ExactSizeIterator<Item = &'a str> + Clone) {
        self.results.clear();
        if self.ids.len() != ids.len() || self.ids.iter().zip(ids.clone()).any(|(a, b)| a != b) {
            self.ids = ids.map(str::to_string).collect();
        }
    }

    fn fill_from_map(&mut self, map: HashMap<String, SimResult<()>>) {
        let mut pairs: Vec<(String, SimResult<()>)> = map.into_iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        self.results.clear();
        self.ids.clear();
        for (id, r) in pairs {
            self.ids.push(id);
            self.results.push(r);
        }
    }
}

/// The orchestrator for a multi-node simulation environment.
///
/// A `World` manages multiple independent `Machine` instances, each with its
/// own address space and clock context, and handles their synchronization.
pub struct World {
    pub name: String,
    pub machines: HashMap<String, Box<dyn MachineTrait>>,
    pub interconnects: Vec<Box<dyn Interconnect>>,
    /// The one UART cross-link medium for this world. Shared by cloning rather
    /// than owned per link, and identical to what the browser attaches, so a
    /// wire behaves the same on either host.
    uart_wires: crate::network::VirtualWireBus,
    /// Serial links on that medium, in manifest order.
    uart_links: Vec<UartLink>,
    next_uart_link_id: u32,
    /// Shared RF medium built from optional env-manifest `rf:` (path loss / RSSI).
    /// Radios attach via their air bus when product wiring is enabled; always
    /// available for inspect / tests when the manifest declared `rf:`.
    pub rf_medium:
        Option<std::sync::Arc<std::sync::Mutex<crate::peripherals::rf_medium::RfMedium>>>,
    /// CPU clock per node (Hz), from the node's system/chip descriptor. The
    /// BLE lockstep reads each node's time off its cycle count with it.
    node_hz: HashMap<String, u64>,
    /// The world's private BLE medium and scripted centrals, when a `ble_air`
    /// or `ble_central` interconnect asked for one. Its presence switches
    /// [`World::step_all`] to time lockstep.
    ble: Option<WorldBle>,
    /// The world's timed UART network, when a `uart_network` interconnect
    /// asked for one. Its presence switches [`World::step_all`] to
    /// conservative time rounds (see [`World::step_all_timed_uart`]).
    uart_net: Option<WorldUartNet>,
    /// The world's GPIO nets, when `gpio_net` interconnects asked for any.
    /// Like a timed UART network they switch [`World::step_all`] to
    /// conservative time rounds.
    gpio: Option<WorldGpio>,
    /// Round clock shared by the timed UART network and the GPIO nets.
    round: RoundClock,
    /// Marker pads per node, collected while interconnects are built and
    /// watched together with the net pads once they all are.
    marker_pins: std::collections::BTreeMap<String, Vec<(String, u8)>>,
    /// `gpio_net` configs, resolved to nets once every interconnect is built.
    pending_nets: Vec<labwired_config::GpioNetConfig>,
    /// Step a world whose only round-based medium is GPIO nets with the
    /// lockstep round driver instead of the per-node one. Tests compare the
    /// two; see [`World::set_gpio_lockstep`].
    gpio_lockstep: bool,
    /// [`World::step_all`]'s results before they become its map, kept from
    /// call to call.
    step_buf: StepResults,
}

/// World time of a round-based world.
#[derive(Debug, Default, Clone, Copy)]
struct RoundClock {
    now_ps: u64,
    /// The round in progress, if one has started and not every node has
    /// reached it yet.
    end_ps: Option<u64>,
}

/// Default upper bound of one timed-network round, ps (100 µs). The round is
/// also bounded by the network's lookahead, so this only matters while no
/// link can carry a character.
pub const UART_NET_DEFAULT_QUANTUM_PS: u64 = 100_000_000;

/// A world's timed UART network and its script.
pub struct WorldUartNet {
    net: crate::network::timed_uart::TimedUartNet,
    /// Scripted events, `(world time ps, event)`, in time order.
    events: Vec<(u64, labwired_config::UartNetworkEvent)>,
    next_event: usize,
    /// Per node: the marker names in watch-channel order and the edge cursor.
    markers: Vec<(String, Vec<String>, u64)>,
    max_quantum_ps: u64,
}

/// Most cycles one node advances in one [`World::step_all`] call of a timed
/// UART world. A round (up to one lookahead, ~82 µs at 115200 baud) is spread
/// over several calls, so one `step_all` stays about as cheap as in any other
/// world — the browser sizes its step batches by calls, not by world time.
pub const UART_NET_STEP_CYCLES: u64 = 128;

/// Longest a node may run ahead of the slowest node of a BLE world, in ns of
/// simulated time. A BLE connection exchanges packets 150 µs apart and a
/// receiver decides "nothing came" 50 µs after its window
/// (`esp32c3::bt_link::RX_DECISION_LAG_NS`); a peer more than that behind
/// would not have transmitted yet. Ten µs leaves room for one instruction's
/// overshoot and the model's 20 µs receive poll.
pub const BLE_LOCKSTEP_QUANTUM_NS: u64 = 10_000;

/// A world's private BLE air and the scripted centrals on it.
pub struct WorldBle {
    air: crate::peripherals::ble_air::BleAirBus,
    attached: std::collections::HashSet<String>,
    centrals: Vec<(String, crate::peripherals::ble_central::ScriptedCentral)>,
    /// Nodes whose last step did not advance their clock (halted, faulted):
    /// left out of the lockstep minimum so they cannot stall the others.
    stalled: std::collections::HashSet<String>,
}

/// One point-to-point serial link between two nodes, as carried on the world's
/// [`crate::network::VirtualWireBus`]. `node_a` sits on side 0, `node_b` side 1.
#[derive(Debug, Clone)]
pub struct UartLink {
    pub id: u32,
    pub node_a: String,
    pub node_b: String,
}

/// One environment node whose browser/host caller has already resolved every
/// filesystem-backed artifact into parsed configuration and firmware bytes.
pub struct ResolvedWorldNode {
    pub id: String,
    pub system: labwired_config::SystemManifest,
    pub chip: labwired_config::ChipDescriptor,
    pub firmware: crate::system::node::NodeFirmware,
}

/// Type-erased trait for machines to allow heterogeneous machines in the world.
pub trait MachineTrait: Send {
    fn name(&self) -> &str;
    fn step(&mut self) -> SimResult<()>;
    fn reset(&mut self) -> SimResult<()>;
    fn total_cycles(&self) -> u64;
    fn read_u8(&self, addr: u64) -> SimResult<u8>;
    fn write_u8(&mut self, addr: u64, val: u8) -> SimResult<()>;
    /// Attach a UART stream device (e.g. a cross-link wire endpoint) to a
    /// named UART peripheral inside this machine.
    fn attach_uart_stream(
        &mut self,
        uart_id: &str,
        dev: Box<dyn crate::peripherals::uart::UartStreamDevice>,
    ) -> anyhow::Result<()>;
    /// Run until the machine's cycle count reaches `target`. The default
    /// single-steps; real machines run batched with idle fast-forward.
    fn advance_to_cycle(&mut self, target: u64) -> SimResult<()> {
        while self.total_cycles() < target {
            let before = self.total_cycles();
            self.step()?;
            if self.total_cycles() == before {
                break;
            }
        }
        Ok(())
    }
    /// Run toward `target` like [`Self::advance_to_cycle`], stopping at the
    /// first boundary at which a `gpio_net` pad of this machine records a
    /// drive change; `true` when it stopped for one. Only a machine whose
    /// [`Self::net_drive_stop_exact`] holds is run this way; the default
    /// never stops early.
    fn advance_to_cycle_or_net_drive_change(&mut self, target: u64) -> SimResult<bool> {
        self.advance_to_cycle(target).map(|()| false)
    }
    /// Whether [`Self::advance_to_cycle_or_net_drive_change`] stops exactly
    /// at the boundary a net pad change is recorded at. A node that cannot is
    /// held to one net latency past its own time, as its own edges come back
    /// to it after that. Default `false`.
    fn net_drive_stop_exact(&self) -> bool {
        false
    }
    /// The cycle before which this machine changes no pad unless one of its
    /// inputs does: its next scheduled event while it sleeps with idle
    /// fast-forward on. `None` (the default) when it may change one at any
    /// time.
    fn idle_quiet_until(&self) -> Option<u64> {
        None
    }
    /// Put the named UART on a timed network link. Default: unsupported.
    fn attach_timed_uart(
        &mut self,
        uart_id: &str,
        _port: crate::network::timed_uart::TimedUartPort,
    ) -> anyhow::Result<()> {
        anyhow::bail!("machine cannot put UART '{uart_id}' on a timed link")
    }
    /// Let timed UARTs schedule wakes for characters now on the wire.
    fn timed_uart_sync(&mut self) {}
    /// Reset the node as its reset pin does (core, NVIC, SysTick, peripheral
    /// reset hooks). Default: the CPU reset.
    fn reset_node(&mut self) -> SimResult<()> {
        self.reset()
    }
    /// Watch GPIO pads `(peripheral id, pin)` for marker edges.
    fn watch_marker_pins(&mut self, _pins: &[(String, u8)]) -> anyhow::Result<()> {
        anyhow::bail!("machine cannot watch marker pins")
    }
    /// Marker edges since `cursor`: `((channel, cycle, level)…, next cursor)`.
    fn marker_edges(&mut self, cursor: u64) -> (Vec<(u32, u64, bool)>, u64) {
        (Vec::new(), cursor)
    }
    /// Put GPIO pads on a `gpio_net`. Call before [`Self::watch_world_pins`].
    fn join_net_pads(&mut self, _pins: &[(String, u8)]) -> anyhow::Result<()> {
        anyhow::bail!("machine cannot join a GPIO net")
    }
    /// Watch marker pads (probes, channels first) and `gpio_net` member pads
    /// (each chip's own drive, the channels after them).
    fn watch_world_pins(
        &mut self,
        _markers: &[(String, u8)],
        _net_pads: &[(String, u8)],
    ) -> anyhow::Result<()> {
        anyhow::bail!("machine cannot watch GPIO net pads")
    }
    /// Four-state drive changes of the watched pads since `cursor`:
    /// `((channel, cycle, state)…, next cursor)`.
    fn net_pad_states(
        &mut self,
        cursor: u64,
    ) -> (Vec<(u32, u64, crate::logic_capture::PadState)>, u64) {
        (Vec::new(), cursor)
    }
    /// Four-state value of each watched pad when the watch was armed.
    fn net_pad_initial_states(&self) -> Vec<Option<crate::logic_capture::PadState>> {
        Vec::new()
    }
    /// Hold a GPIO pad at `level` as an external driver would.
    fn drive_gpio_input(&mut self, _peripheral: &str, _pin: u8, _level: bool) -> bool {
        false
    }
    /// Arm an instrument's watch set next to the world's own pad watches.
    /// Default: unsupported (every ref reports why).
    fn observer_watch(
        &mut self,
        refs: &[crate::machine::world_hooks::ObserverRef],
    ) -> Vec<crate::machine::world_hooks::ObserverRow> {
        refs.iter()
            .map(|_| crate::machine::world_hooks::ObserverRow {
                initial: None,
                error: Some("this machine has no instrument watch".to_string()),
            })
            .collect()
    }
    /// The instrument's level edges since `cursor`.
    fn observer_edges(&mut self, cursor: u64) -> crate::logic_capture::LogicEdgeBatch {
        crate::logic_capture::LogicEdgeBatch {
            cursor,
            dropped: 0,
            edges: Vec::new(),
        }
    }
    /// The instrument's four-state edges since `cursor`, and each channel's
    /// state when it was armed.
    fn observer_states(
        &mut self,
        cursor: u64,
    ) -> (
        crate::logic_capture::LogicStateBatch,
        Vec<Option<crate::logic_capture::PadState>>,
    ) {
        (
            crate::logic_capture::LogicStateBatch {
                cursor,
                dropped: 0,
                edges: Vec::new(),
            },
            Vec::new(),
        )
    }
    /// The level each ref reads now.
    fn observer_sample(
        &self,
        refs: &[crate::machine::world_hooks::ObserverRef],
    ) -> Vec<Option<bool>> {
        vec![None; refs.len()]
    }
    /// A GPIO pin's output latch (`output`) or input level.
    fn gpio_level(&self, _peripheral: &str, _pin: u8, _output: bool) -> Option<bool> {
        None
    }
    /// Let this node skip idle time (a core parked in WFI or `SLEEP`) when
    /// it advances. Off by default, as for a lone machine.
    fn set_idle_fast_forward(&mut self, _enabled: bool) {}
    /// Cycles this node skipped through idle fast-forward so far.
    fn idle_fast_forward_cycles(&self) -> u64 {
        0
    }
    /// `true` while a watched pad of this node (a marker, a `gpio_net`
    /// member) is on the per-cycle poll capture path, which runs the node one
    /// instruction at a time and turns idle fast-forward off.
    fn logic_poll_active(&self) -> bool {
        false
    }
    /// Drive a simulated input channel (a sensor's temperature, a distance).
    fn set_input_channel(&mut self, _channel: &str, _value: f64) -> Result<(), String> {
        Err("this machine has no input channels".to_string())
    }
    /// The input channels the attached devices expose.
    fn list_input_channels(&mut self) -> Vec<(String, crate::sim_input::InputChannel)> {
        Vec::new()
    }
    /// True if this machine hosts a Quectel BG770A (needs lab AirBus).
    fn has_cellular_modem(&self) -> bool {
        false
    }
    /// Bind nRF/BLE/cellular peers to a shared lab air. Default no-op for mocks.
    fn attach_lab_air(
        &mut self,
        _node_id: &str,
        _nrf: crate::peripherals::nrf52::radio::VirtualAirBus,
        _ble: crate::peripherals::ble_air::BleAirBus,
        _cellular: crate::network::SimMqttFabric,
    ) {
    }
    /// Move this machine's BLE controllers onto `air` (a world's private BLE
    /// medium). Default no-op for mocks.
    fn attach_ble_air(&mut self, _air: crate::peripherals::ble_air::BleAirBus) {}
    /// Attach a per-node UART capture sink. The default is intentionally a
    /// no-op so existing third-party/mock `MachineTrait` implementations stay
    /// source-compatible; real [`Machine`] instances wire every console UART.
    fn attach_uart_tx_sink(
        &mut self,
        _sink: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        _echo_stdout: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    /// Prefix this machine's UART console output, so a world's shared stdout
    /// stays readable per node. Default no-op keeps third-party mock machines
    /// source-compatible.
    fn set_stdout_prefix(&mut self, _prefix: &str) {}
    /// Return a final machine snapshot for a world artifact. Mocks that do not
    /// model state may retain the default `None`; concrete machines provide the
    /// complete snapshot.
    fn snapshot(&self) -> Option<crate::snapshot::MachineSnapshot> {
        None
    }
    fn display_artifact(
        &self,
        _device_id: &str,
        _include_bytes: bool,
    ) -> Option<crate::inspect::Artifact> {
        None
    }
    fn bus_trace_snapshot(&self) -> Vec<crate::bus::BusTraceEvent> {
        Vec::new()
    }
    fn get_pc(&self) -> u32 {
        0
    }
    fn get_register(&self, _id: usize) -> u32 {
        0
    }
    fn get_register_names(&self) -> Vec<String> {
        Vec::new()
    }
    fn read_memory(&self, _addr: u32, _len: usize) -> SimResult<Vec<u8>> {
        Err(crate::SimulationError::NotImplemented(
            "memory inspection".into(),
        ))
    }
    /// Attach one endpoint of a `CanBus` to a named FDCAN peripheral. The
    /// default keeps third-party mock machines source-compatible while making
    /// an unsupported topology error explicit.
    fn attach_can_bus(
        &mut self,
        can_id: &str,
        _tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
        _rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        anyhow::bail!(
            "machine '{}' cannot attach CAN bus endpoint '{can_id}'",
            self.name()
        )
    }
}

impl<C: Cpu + 'static> MachineTrait for Machine<C> {
    fn name(&self) -> &str {
        // We might need to add a name field to Machine or handle mapping in World
        "unnamed"
    }

    fn step(&mut self) -> SimResult<()> {
        self.step()
    }

    fn reset(&mut self) -> SimResult<()> {
        self.reset()
    }

    fn total_cycles(&self) -> u64 {
        self.total_cycles
    }

    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        self.bus.read_u8(addr)
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> SimResult<()> {
        self.bus.write_u8(addr, val)
    }

    fn attach_uart_stream(
        &mut self,
        uart_id: &str,
        dev: Box<dyn crate::peripherals::uart::UartStreamDevice>,
    ) -> anyhow::Result<()> {
        self.bus.attach_uart_stream_by_id(uart_id, dev)
    }

    fn has_cellular_modem(&self) -> bool {
        self.bus.has_cellular_modem()
    }

    fn attach_lab_air(
        &mut self,
        node_id: &str,
        nrf: crate::peripherals::nrf52::radio::VirtualAirBus,
        ble: crate::peripherals::ble_air::BleAirBus,
        cellular: crate::network::SimMqttFabric,
    ) {
        self.bus.attach_lab_air(node_id, nrf, ble, cellular);
    }

    fn attach_ble_air(&mut self, air: crate::peripherals::ble_air::BleAirBus) {
        self.bus.attach_ble_air(air);
    }

    fn attach_uart_tx_sink(
        &mut self,
        sink: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        echo_stdout: bool,
    ) -> anyhow::Result<()> {
        // AVR USART TX is modelled on the CPU, not a bus UART peripheral.
        if let Some(avr) = self
            .cpu
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<crate::cpu::Avr>())
        {
            avr.set_serial_sink(sink.clone());
        }
        self.bus.attach_uart_tx_sink(sink, echo_stdout);
        Ok(())
    }

    fn set_stdout_prefix(&mut self, prefix: &str) {
        for p in self.bus.peripherals.iter_mut() {
            if let Some(uart) = p
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<crate::peripherals::uart::Uart>())
            {
                uart.set_stdout_prefix(prefix.to_string());
            }
        }
    }

    fn snapshot(&self) -> Option<crate::snapshot::MachineSnapshot> {
        Some(Machine::snapshot(self))
    }

    fn display_artifact(
        &self,
        device_id: &str,
        include_bytes: bool,
    ) -> Option<crate::inspect::Artifact> {
        self.bus.display_artifact(
            device_id,
            &crate::inspect::InspectOpts {
                include_bytes,
                peripheral: None,
            },
        )
    }

    fn bus_trace_snapshot(&self) -> Vec<crate::bus::BusTraceEvent> {
        self.bus.bus_trace_snapshot()
    }

    fn get_pc(&self) -> u32 {
        self.cpu.get_pc()
    }
    fn get_register(&self, id: usize) -> u32 {
        self.cpu.get_register(id as u8)
    }
    fn get_register_names(&self) -> Vec<String> {
        self.cpu.get_register_names()
    }
    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>> {
        crate::DebugControl::read_memory(self, addr, len)
    }

    fn attach_can_bus(
        &mut self,
        can_id: &str,
        tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
        rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        self.bus.attach_can_endpoint_by_id(can_id, tx, rx)
    }

    fn advance_to_cycle(&mut self, target: u64) -> SimResult<()> {
        Machine::advance_to_cycle(self, target)
    }

    fn advance_to_cycle_or_net_drive_change(&mut self, target: u64) -> SimResult<bool> {
        Machine::advance_to_cycle_or_net_drive_change(self, target)
    }

    fn net_drive_stop_exact(&self) -> bool {
        Machine::net_drive_stop_exact(self)
    }

    fn idle_quiet_until(&self) -> Option<u64> {
        Machine::idle_quiet_until(self)
    }

    fn set_idle_fast_forward(&mut self, enabled: bool) {
        self.config.idle_fast_forward_enabled = enabled;
    }

    fn idle_fast_forward_cycles(&self) -> u64 {
        self.idle_fast_forward_cycles_skipped
    }

    fn logic_poll_active(&self) -> bool {
        Machine::logic_poll_active(self)
    }

    fn attach_timed_uart(
        &mut self,
        uart_id: &str,
        port: crate::network::timed_uart::TimedUartPort,
    ) -> anyhow::Result<()> {
        self.bus.attach_timed_uart_by_id(uart_id, port)
    }

    fn timed_uart_sync(&mut self) {
        Machine::timed_uart_sync(self);
    }

    fn reset_node(&mut self) -> SimResult<()> {
        Machine::reset_node(self)
    }

    fn watch_marker_pins(&mut self, pins: &[(String, u8)]) -> anyhow::Result<()> {
        Machine::watch_marker_pins(self, pins)
    }

    fn marker_edges(&mut self, cursor: u64) -> (Vec<(u32, u64, bool)>, u64) {
        Machine::marker_edges(self, cursor)
    }

    fn join_net_pads(&mut self, pins: &[(String, u8)]) -> anyhow::Result<()> {
        Machine::join_net_pads(self, pins)
    }
    fn watch_world_pins(
        &mut self,
        markers: &[(String, u8)],
        net_pads: &[(String, u8)],
    ) -> anyhow::Result<()> {
        Machine::watch_world_pins(self, markers, net_pads)
    }

    fn net_pad_states(
        &mut self,
        cursor: u64,
    ) -> (Vec<(u32, u64, crate::logic_capture::PadState)>, u64) {
        Machine::net_pad_states(self, cursor)
    }

    fn net_pad_initial_states(&self) -> Vec<Option<crate::logic_capture::PadState>> {
        Machine::net_pad_initial_states(self)
    }

    fn drive_gpio_input(&mut self, peripheral: &str, pin: u8, level: bool) -> bool {
        Machine::drive_gpio_input(self, peripheral, pin, level)
    }

    fn observer_watch(
        &mut self,
        refs: &[crate::machine::world_hooks::ObserverRef],
    ) -> Vec<crate::machine::world_hooks::ObserverRow> {
        Machine::observer_watch(self, refs)
    }

    fn observer_edges(&mut self, cursor: u64) -> crate::logic_capture::LogicEdgeBatch {
        Machine::observer_edges(self, cursor)
    }

    fn observer_states(
        &mut self,
        cursor: u64,
    ) -> (
        crate::logic_capture::LogicStateBatch,
        Vec<Option<crate::logic_capture::PadState>>,
    ) {
        Machine::observer_states(self, cursor)
    }

    fn observer_sample(
        &self,
        refs: &[crate::machine::world_hooks::ObserverRef],
    ) -> Vec<Option<bool>> {
        Machine::observer_sample(self, refs)
    }

    fn gpio_level(&self, peripheral: &str, pin: u8, output: bool) -> Option<bool> {
        Machine::gpio_level(self, peripheral, pin, output)
    }

    fn set_input_channel(&mut self, channel: &str, value: f64) -> Result<(), String> {
        Machine::set_input(self, channel, value).map_err(|e| e.to_string())
    }

    fn list_input_channels(&mut self) -> Vec<(String, crate::sim_input::InputChannel)> {
        Machine::list_inputs(self)
    }
}

impl World {
    pub fn new(name: String) -> Self {
        Self {
            name,
            machines: HashMap::new(),
            interconnects: Vec::new(),
            uart_wires: crate::network::VirtualWireBus::new(),
            uart_links: Vec::new(),
            next_uart_link_id: 0,
            rf_medium: None,
            node_hz: HashMap::new(),
            ble: None,
            uart_net: None,
            gpio: None,
            round: RoundClock::default(),
            marker_pins: Default::default(),
            pending_nets: Vec::new(),
            gpio_lockstep: false,
            step_buf: StepResults::new(),
        }
    }

    /// Record node `id`'s CPU clock, for the BLE time lockstep.
    pub fn set_node_hz(&mut self, id: &str, hz: u64) {
        if hz > 0 {
            self.node_hz.insert(id.to_string(), hz);
        }
    }

    /// The CPU clock of node `id`, in Hz.
    pub fn node_hz(&self, id: &str) -> Option<u64> {
        self.node_hz.get(id).copied()
    }

    /// Simulated time of node `id` in ns, if its clock is known.
    pub fn node_time_ns(&self, id: &str) -> Option<u64> {
        let hz = *self.node_hz.get(id)?;
        let cycles = self.machines.get(id)?.total_cycles();
        Some((u128::from(cycles) * 1_000_000_000 / u128::from(hz)) as u64)
    }

    /// The world's private BLE air, if it has one.
    pub fn ble_air(&self) -> Option<&crate::peripherals::ble_air::BleAirBus> {
        self.ble.as_ref().map(|b| &b.air)
    }

    /// Every scripted central's report, in manifest order, keyed by its id.
    pub fn ble_central_reports(
        &self,
    ) -> Vec<(String, crate::peripherals::ble_central::CentralReport)> {
        self.ble
            .as_ref()
            .map(|b| {
                b.centrals
                    .iter()
                    .map(|(id, c)| (id.clone(), c.report()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Put `nodes` on the world's private BLE air (created on first use) and
    /// switch the world to time lockstep.
    pub fn attach_ble_nodes(&mut self, nodes: &[String]) -> anyhow::Result<()> {
        if self.uart_net.is_some() {
            anyhow::bail!("a world cannot have both a BLE air and a timed uart_network yet");
        }
        for id in nodes {
            if !self.machines.contains_key(id) {
                anyhow::bail!("unknown node '{id}'");
            }
            if !self.node_hz.contains_key(id) {
                anyhow::bail!("node '{id}' has no known CPU clock; a BLE world steps by time");
            }
        }
        let ble = self.ble.get_or_insert_with(|| WorldBle {
            air: crate::peripherals::ble_air::BleAirBus::new(),
            attached: Default::default(),
            centrals: Vec::new(),
            stalled: Default::default(),
        });
        for id in nodes {
            if ble.attached.insert(id.clone()) {
                self.machines
                    .get_mut(id)
                    .expect("validated above")
                    .attach_ble_air(ble.air.clone());
            }
        }
        Ok(())
    }

    /// Add a scripted central on the world's BLE air (see
    /// [`crate::peripherals::ble_central`]).
    pub fn add_ble_central(
        &mut self,
        id: String,
        cfg: crate::peripherals::ble_central::CentralConfig,
    ) -> anyhow::Result<()> {
        let Some(ble) = self.ble.as_mut() else {
            anyhow::bail!("ble_central '{id}': the world has no BLE air");
        };
        if ble.centrals.iter().any(|(existing, _)| *existing == id) {
            anyhow::bail!("duplicate ble_central id '{id}'");
        }
        let central = crate::peripherals::ble_central::ScriptedCentral::new(ble.air.clone(), cfg);
        ble.centrals.push((id, central));
        Ok(())
    }

    /// This world's serial links, in manifest order.
    pub fn uart_links(&self) -> &[UartLink] {
        &self.uart_links
    }

    /// The medium carrying this world's serial links — used to inject wire
    /// faults (see [`crate::network::VirtualWireBus::corrupt_next`]).
    pub fn uart_wires(&self) -> &crate::network::VirtualWireBus {
        &self.uart_wires
    }

    pub fn add_machine(&mut self, id: String, machine: Box<dyn MachineTrait>) {
        self.machines.insert(id, machine);
    }

    pub fn add_interconnect(&mut self, interconnect: Box<dyn Interconnect>) {
        self.interconnects.push(interconnect);
    }

    /// Step all machines in the world.
    ///
    /// This is the simplest synchronization strategy: step every machine once.
    /// Future improvements will include Global Virtual Time (GVT) and
    /// Chandy-Lamport for distributed snapshots.
    pub fn step_all(&mut self) -> HashMap<String, SimResult<()>> {
        if self.ble.is_some() {
            return self.step_all_time_lockstep();
        }
        if self.gpio_per_node() {
            let mut results = std::mem::take(&mut self.step_buf);
            self.step_all_gpio(&mut results);
            let map = results.drain_to_map();
            self.step_buf = results;
            return map;
        }
        if self.uart_net.is_some() || self.gpio.is_some() {
            return self.step_all_rounds();
        }
        let mut results = HashMap::new();
        let mut ids: Vec<_> = self.machines.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let result = self
                .machines
                .get_mut(&id)
                .expect("machine id was collected from this world")
                .step();
            results.insert(id, result);
        }
        for interconnect in &mut self.interconnects {
            if let Err(e) = interconnect.tick() {
                tracing::warn!("interconnect error: {:?}", e);
            }
        }
        results
    }

    /// [`World::step_all`] into `results`, which keeps its storage from one
    /// call to the next: the same per-machine results in machine id order,
    /// without a map and a copy of every id per call. A front end that steps
    /// a world round by round (`labwired test`, Python) reuses one.
    pub fn step_all_into(&mut self, results: &mut StepResults) {
        if self.ble.is_none() && self.gpio_per_node() {
            self.step_all_gpio(results);
        } else {
            let map = self.step_all();
            results.fill_from_map(map);
        }
    }

    /// GPIO nets and no timed UART network: `step_all` runs the per-node
    /// scheduler.
    fn gpio_per_node(&self) -> bool {
        self.gpio.is_some() && self.uart_net.is_none() && !self.gpio_lockstep
    }

    /// One round of a BLE world: step every node that is not more than
    /// [`BLE_LOCKSTEP_QUANTUM_NS`] ahead of the slowest one, then advance the
    /// scripted centrals to the slowest node's time. Nodes run by simulated
    /// time rather than by instruction count here, so a node that sleeps in a
    /// fast-forwarded idle loop cannot run milliseconds past its peer and miss
    /// a packet that is due 150 µs after its own.
    fn step_all_time_lockstep(&mut self) -> HashMap<String, SimResult<()>> {
        let mut ids: Vec<_> = self.machines.keys().cloned().collect();
        ids.sort();
        let stalled = self
            .ble
            .as_ref()
            .map(|b| b.stalled.clone())
            .unwrap_or_default();
        let min_t = ids
            .iter()
            .filter(|id| !stalled.contains(*id))
            .filter_map(|id| self.node_time_ns(id))
            .min()
            .unwrap_or(0);
        let mut results = HashMap::new();
        let mut now_stalled = Vec::new();
        let mut now_running = Vec::new();
        for id in ids.iter() {
            let t = self.node_time_ns(id);
            if t.is_some_and(|t| t > min_t + BLE_LOCKSTEP_QUANTUM_NS) {
                // Ahead of the slowest node: wait for it this round.
                results.insert(id.clone(), Ok(()));
                continue;
            }
            let machine = self
                .machines
                .get_mut(id)
                .expect("machine id was collected from this world");
            let before = machine.total_cycles();
            let result = machine.step();
            if machine.total_cycles() == before {
                now_stalled.push(id.clone());
            } else {
                now_running.push(id.clone());
            }
            results.insert(id.clone(), result);
        }
        for interconnect in &mut self.interconnects {
            if let Err(e) = interconnect.tick() {
                tracing::warn!("interconnect error: {:?}", e);
            }
        }
        let floor = ids
            .iter()
            .filter(|id| !now_stalled.contains(*id))
            .filter_map(|id| self.node_time_ns(id))
            .min();
        if let Some(ble) = self.ble.as_mut() {
            for id in now_stalled {
                ble.stalled.insert(id);
            }
            for id in now_running {
                ble.stalled.remove(&id);
            }
            if let Some(t) = floor {
                for (_, central) in ble.centrals.iter_mut() {
                    central.advance_to(t);
                }
            }
        }
        results
    }

    /// One slice of a conservative synchronisation round of a round-based
    /// world (a timed UART network, GPIO nets, or both): every node advances
    /// at most [`UART_NET_STEP_CYCLES`] toward the round end, and the round
    /// completes when all have reached it.
    ///
    /// Every node runs to the same world time `T + Δ`, where `Δ` is at most
    /// the shortest time anything one node does can take to reach another:
    /// for the UART network the lookahead (a start bit leaving any sender to
    /// any receiver acting on that character), for GPIO nets the shortest net
    /// latency. What a node puts on a wire in this round therefore cannot be
    /// due at a peer before the round ends, so the peer learns of it (at the
    /// start of the next round, `timed_uart_sync`; or when the round
    /// completes, for GPIO edges) before its time comes — and acts on it at
    /// the exact cycle, whatever order the nodes run in. Results do not
    /// depend on `Δ` or on node order. Scripted events are applied at round
    /// boundaries, and a round never crosses the next one.
    fn step_all_rounds(&mut self) -> HashMap<String, SimResult<()>> {
        use crate::network::timed_uart::ps_to_cycles_ceil;
        let mut ids: Vec<_> = self.machines.keys().cloned().collect();
        ids.sort();
        let mut results = HashMap::new();
        let target = match self.round.end_ps {
            Some(target) => target,
            None => self.start_round(&ids, &mut results),
        };
        let mut all_there = true;
        let mut reached = std::collections::BTreeSet::new();
        for id in &ids {
            let hz = self.node_hz.get(id).copied().unwrap_or(0);
            let machine = self
                .machines
                .get_mut(id)
                .expect("machine id was collected from this world");
            if hz == 0 {
                results.entry(id.clone()).or_insert(Ok(()));
                continue;
            }
            let cycle = ps_to_cycles_ceil(target, hz);
            let before = machine.total_cycles();
            let r = match self.gpio.as_mut() {
                None => {
                    if before < cycle {
                        machine.advance_to_cycle(cycle.min(before + UART_NET_STEP_CYCLES))
                    } else {
                        Ok(())
                    }
                }
                Some(g) => g.advance_node(id, machine.as_mut(), cycle),
            };
            let after = machine.total_cycles();
            // A node that made no progress (halted, locked up) cannot hold
            // the round open for the others.
            if after < cycle && after > before {
                all_there = false;
            }
            if after >= cycle {
                reached.insert(id.clone());
            }
            results.entry(id.clone()).or_insert(r);
        }
        if all_there {
            self.round.now_ps = target;
            self.round.end_ps = None;
        }
        for interconnect in &mut self.interconnects {
            if let Err(e) = interconnect.tick() {
                tracing::warn!("interconnect error: {:?}", e);
            }
        }
        // Marker edges onto the one timeline.
        if let Some(st) = self.uart_net.as_mut() {
            use crate::network::timed_uart::cycles_to_ps;
            for (node, names, cursor) in st.markers.iter_mut() {
                let hz = self.node_hz.get(node).copied().unwrap_or(0);
                let Some(machine) = self.machines.get_mut(node) else {
                    continue;
                };
                let (edges, next) = machine.marker_edges(*cursor);
                *cursor = next;
                for (ch, cycle, level) in edges {
                    // Net pads share the node's watch set after the markers.
                    let Some(name) = names.get(ch as usize) else {
                        continue;
                    };
                    st.net
                        .record_marker(node, name, level, cycles_to_ps(cycle, hz));
                }
            }
        }
        // GPIO nets: merge what the nodes drove into the nets, up to the
        // time every node has reached.
        if all_there {
            if let Some(g) = self.gpio.as_mut() {
                g.merge_edges(&mut self.machines, &reached);
            }
        }
        results
    }

    /// One slice of a round of a world whose nets are GPIO nets only: the
    /// same rounds, round ends and per-call step budget as
    /// [`World::step_all_rounds`], run by the per-node scheduler
    /// ([`gpio_nets::WorldGpio::run_to`]) with none of the lockstep driver's
    /// per-round lookups and allocations.
    fn step_all_gpio(&mut self, results: &mut StepResults) {
        let target = match self.round.end_ps {
            Some(target) => target,
            // Without a timed UART network a round start has no events, no
            // resets and so no results of its own.
            None => self.start_round(&[], &mut HashMap::new()),
        };
        let out = self.gpio.as_mut().expect("gpio world").run_to(
            &mut self.machines,
            &self.node_hz,
            target,
            Some(UART_NET_STEP_CYCLES),
            results,
        );
        if out.all_there {
            self.round.now_ps = target;
            self.round.end_ps = None;
        }
        for interconnect in &mut self.interconnects {
            if let Err(e) = interconnect.tick() {
                tracing::warn!("interconnect error: {:?}", e);
            }
        }
    }

    /// Run a round-based world (timed UART network or GPIO nets) to world
    /// time `target_ps` in one call; `None` for a world without a round clock.
    ///
    /// Same results as calling [`World::step_all`] until
    /// [`World::round_now_ps`] reaches `target_ps`. A world whose only
    /// round-based medium is GPIO nets and that has no other interconnect to
    /// tick runs every node to exactly `target_ps` (the first instruction
    /// boundary at or after it) with the per-node scheduler, each node in as
    /// few pieces as the nets' latencies allow and with no per-round call
    /// overhead. Any other round-based world is stepped round by round and
    /// stops at the first round end at or after `target_ps`. Stops at the
    /// first round in which a node returns an error.
    pub fn run_until_ps(&mut self, target_ps: u64) -> Option<HashMap<String, SimResult<()>>> {
        if self.uart_net.is_none() && self.gpio.is_none() {
            return None;
        }
        let fast = self.gpio.is_some()
            && self.uart_net.is_none()
            && !self.gpio_lockstep
            && self.interconnects.is_empty();
        if fast {
            // Finish a round a step budget left part way, so the round clock
            // stays on round ends, then run the rest in one go.
            let mut results = StepResults::new();
            while self.round.end_ps.is_some_and(|end| end <= target_ps) {
                self.step_all_gpio(&mut results);
                if results.first_error().is_some() {
                    return Some(results.into_map());
                }
            }
            if self.round.end_ps.is_none() && self.round.now_ps < target_ps {
                let out = self.gpio.as_mut().expect("gpio world").run_to(
                    &mut self.machines,
                    &self.node_hz,
                    target_ps,
                    None,
                    &mut results,
                );
                if out.all_there {
                    self.round.now_ps = target_ps;
                }
                return Some(results.into_map());
            }
        }
        let mut results = HashMap::new();
        // A round ends once every node got there or made no progress, so the
        // round clock always moves and this ends.
        while self.round.now_ps < target_ps {
            results = self.step_all();
            if results.values().any(Result::is_err) {
                break;
            }
        }
        if results.is_empty() {
            results = self
                .machines
                .keys()
                .map(|id| (id.clone(), Ok(())))
                .collect();
        }
        Some(results)
    }

    /// `rounds` calls of [`World::step_all`], in one call when the world
    /// allows: same results, same round clock. Returns the last call's
    /// results, or the first one with an error.
    ///
    /// A world whose only round-based medium is GPIO nets, with no other
    /// interconnect to tick and no round in flight, and whose rounds each
    /// fit in one call's step budget, runs the `rounds` rounds as one
    /// [`World::run_until_ps`]. Any other world is stepped call by call.
    pub fn step_rounds(&mut self, rounds: u64) -> HashMap<String, SimResult<()>> {
        if let Some(round_ps) = self.batchable_round_ps() {
            let target = self
                .round
                .now_ps
                .saturating_add(round_ps.saturating_mul(rounds));
            if let Some(results) = self.run_until_ps(target) {
                return results;
            }
        }
        let mut results = HashMap::new();
        for _ in 0..rounds {
            results = self.step_all();
            if results.values().any(Result::is_err) {
                break;
            }
        }
        results
    }

    /// The length of every round of a world in which a call of
    /// [`World::step_all`] always completes one round and a run of rounds
    /// can be one [`World::run_until_ps`]; `None` for any other world.
    fn batchable_round_ps(&self) -> Option<u64> {
        let gpio = self.gpio.as_ref()?;
        if self.uart_net.is_some()
            || self.gpio_lockstep
            || !self.interconnects.is_empty()
            || self.round.end_ps.is_some()
        {
            return None;
        }
        // The round `start_round` opens without a timed UART network.
        let round_ps = gpio.round_ps.min(UART_NET_DEFAULT_QUANTUM_PS);
        if round_ps < 1_000 {
            return None;
        }
        // Each node gets there inside one call's step budget (with room for
        // an instruction that overshot the last round end).
        let max_hz = self
            .machines
            .keys()
            .filter_map(|id| self.node_hz.get(id))
            .max();
        let cycles = crate::network::timed_uart::ps_to_cycles_ceil(round_ps, *max_hz?);
        (cycles + 16 <= UART_NET_STEP_CYCLES).then_some(round_ps)
    }

    /// Open the next round: apply the scripted events due at its start, reset
    /// nodes, fix its end at one lookahead (or the next event), and let every
    /// timed USART see the characters now on the wire. Returns the round end.
    fn start_round(&mut self, ids: &[String], results: &mut HashMap<String, SimResult<()>>) -> u64 {
        let t = self.round.now_ps;
        let mut resets = Vec::new();
        let mut lookahead = u64::MAX;
        let mut quantum = UART_NET_DEFAULT_QUANTUM_PS;
        let mut next_event = None;
        if let Some(st) = self.uart_net.as_mut() {
            while st.next_event < st.events.len() && st.events[st.next_event].0 <= t {
                let (_, ev) = st.events[st.next_event].clone();
                st.next_event += 1;
                if let Some(link) = ev.slow_link {
                    let delay = ev.delay_us.map(us_to_ps);
                    let current = st.net.report(u64::MAX, t).links[link as usize].delay_ps;
                    st.net.set_link_delay(
                        link as usize,
                        delay.unwrap_or(current),
                        ev.jitter_us.map(us_to_ps),
                        t,
                    );
                } else if let Some(link) = ev.cut_link {
                    st.net.set_link_connected(link as usize, false, t);
                } else if let Some(link) = ev.restore_link {
                    st.net.set_link_connected(link as usize, true, t);
                } else if let Some(node) = ev.reset_node {
                    st.net.record_node_reset(&node, t);
                    resets.push(node);
                }
            }
            lookahead = st.net.lookahead_ps().unwrap_or(st.max_quantum_ps);
            quantum = st.max_quantum_ps;
            next_event = st.events.get(st.next_event).map(|(at, _)| *at);
        }
        if let Some(g) = self.gpio.as_ref() {
            lookahead = lookahead.min(g.round_ps);
        }
        let mut target = t + lookahead.min(quantum).max(1_000);
        if let Some(at) = next_event {
            if at > t {
                target = target.min(at);
            }
        }
        self.round.end_ps = Some(target);
        for node in resets {
            if let Some(m) = self.machines.get_mut(&node) {
                if let Err(e) = m.reset_node() {
                    results.insert(node.clone(), Err(e));
                }
            }
        }
        if self.uart_net.is_some() {
            for id in ids {
                if let Some(m) = self.machines.get_mut(id) {
                    m.timed_uart_sync();
                }
            }
        }
        target
    }

    /// World time a timed UART world has reached, ps.
    pub fn uart_network_now_ps(&self) -> Option<u64> {
        self.uart_net.as_ref().map(|_| self.round.now_ps)
    }

    /// World time a round-based world (timed UART network or GPIO nets) has
    /// reached, ps; `None` for any other world.
    pub fn round_now_ps(&self) -> Option<u64> {
        (self.uart_net.is_some() || self.gpio.is_some()).then_some(self.round.now_ps)
    }

    /// Every net delivery applied to a pad so far (node, pad, the cycle it
    /// was applied at, the level), capped at 65 536, ordered by the time the
    /// wire carried the change, then node id. For tests and tools.
    pub fn gpio_net_applied(&self) -> &[AppliedDelivery] {
        self.gpio.as_ref().map_or(&[], |g| g.applied())
    }

    /// How the per-node GPIO-net scheduler has cut the run so far: how many
    /// pieces the net nodes ran in, and how many of those ended at the node's
    /// own drive change. For tests and tools; `None` without GPIO nets.
    pub fn gpio_net_scheduler_stats(&self) -> Option<GpioSchedulerStats> {
        self.gpio.as_ref().map(|g| g.stats)
    }

    /// Run GPIO-net rounds shorter than the shortest latency. Results must
    /// not change; this exists so tests can prove it.
    #[doc(hidden)]
    pub fn set_gpio_round_ps(&mut self, round_ps: u64) -> anyhow::Result<()> {
        match self.gpio.as_mut() {
            Some(g) => g.set_round_ps(round_ps),
            None => anyhow::bail!("this world has no gpio_net"),
        }
    }

    /// Step GPIO-net worlds with the lockstep round driver (the one a world
    /// with a timed UART network uses) instead of the per-node scheduler.
    /// Results must not change; this exists so tests can prove it.
    #[doc(hidden)]
    pub fn set_gpio_lockstep(&mut self, lockstep: bool) {
        self.gpio_lockstep = lockstep;
    }

    /// Every GPIO net's state, counters and diagnostics (`GPIO_NET_CONTENTION`,
    /// `GPIO_NET_FLOATING`), in manifest order. Empty without `gpio_net`s.
    pub fn gpio_net_reports(&self) -> Vec<crate::network::gpio_net::GpioNetReport> {
        self.gpio
            .as_ref()
            .map(|g| g.nets.iter().map(|n| n.report()).collect())
            .unwrap_or_default()
    }

    /// The timed UART network's statistics, tagged messages, and timeline
    /// events from sequence number `since` on.
    pub fn uart_network_report(&self, since: u64) -> Option<crate::network::timed_uart::NetReport> {
        self.uart_net
            .as_ref()
            .map(|n| n.net.report(since, self.round.now_ps))
    }

    /// The timed UART medium itself (tests inject faults through it).
    pub fn uart_network(&self) -> Option<&crate::network::timed_uart::TimedUartNet> {
        self.uart_net.as_ref().map(|n| &n.net)
    }

    /// Build the `uart_network` interconnect: links, script and markers.
    fn build_uart_network(
        &mut self,
        ic: &labwired_config::InterconnectConfig,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        use labwired_config::{UartNetworkConfig, UartTopology};
        if self.uart_net.is_some() {
            anyhow::bail!("a world has at most one uart_network");
        }
        if self.ble.is_some() {
            anyhow::bail!("a world cannot have both a BLE air and a timed uart_network yet");
        }
        let cfg = UartNetworkConfig::from_interconnect_config(&ic.config)?;
        for id in &ic.nodes {
            if !self.machines.contains_key(id) {
                anyhow::bail!("unknown node '{id}'");
            }
            if !self.node_hz.contains_key(id) {
                anyhow::bail!("node '{id}' has no known CPU clock; a timed network steps by time");
            }
        }
        let pairs: Vec<((String, String), (String, String))> = match cfg.topology {
            UartTopology::Chain => ic
                .nodes
                .windows(2)
                .map(|w| {
                    (
                        (w[0].clone(), cfg.uart_out.clone()),
                        (w[1].clone(), cfg.uart_in.clone()),
                    )
                })
                .collect(),
            UartTopology::Star => {
                let hub = cfg.hub.clone().unwrap_or_else(|| ic.nodes[0].clone());
                if !ic.nodes.contains(&hub) {
                    anyhow::bail!("star hub '{hub}' is not one of the listed nodes");
                }
                let spokes: Vec<_> = ic.nodes.iter().filter(|n| **n != hub).collect();
                if cfg.hub_uarts.len() != spokes.len() {
                    anyhow::bail!(
                        "star: hub_uarts lists {} UARTs for {} spokes",
                        cfg.hub_uarts.len(),
                        spokes.len()
                    );
                }
                if !cfg.spoke_uarts.is_empty() && cfg.spoke_uarts.len() != spokes.len() {
                    anyhow::bail!(
                        "star: spoke_uarts lists {} UARTs for {} spokes",
                        cfg.spoke_uarts.len(),
                        spokes.len()
                    );
                }
                spokes
                    .into_iter()
                    .enumerate()
                    .map(|(k, spoke)| {
                        let spoke_uart = cfg.spoke_uarts.get(k).unwrap_or(&cfg.spoke_uart).clone();
                        (
                            (hub.clone(), cfg.hub_uarts[k].clone()),
                            (spoke.clone(), spoke_uart),
                        )
                    })
                    .collect()
            }
        };
        let mut used = std::collections::HashSet::new();
        for (a, b) in &pairs {
            for end in [a, b] {
                if !used.insert(end.clone()) {
                    anyhow::bail!("UART '{}' of node '{}' is on two links", end.1, end.0);
                }
            }
        }
        let net = crate::network::timed_uart::TimedUartNet::new();
        net.set_message_tagging(cfg.messages.as_ref().map(|m| {
            crate::network::timed_uart::MessageTagging {
                sync: m.sync,
                length: m.length,
                id_offset: m.id_offset,
                id_bytes: m.id_bytes,
                hop_offset: m.hop_offset,
                checksum_xor: m.checksum_xor,
            }
        }));
        for ((na, ua), (nb, ub)) in &pairs {
            let (ha, hb) = (self.node_hz[na], self.node_hz[nb]);
            let (_, pa, pb) = net.add_link(
                (na, ua, ha),
                (nb, ub, hb),
                us_to_ps(cfg.delay_us),
                us_to_ps(cfg.jitter_us),
                cfg.seed,
            );
            self.machines
                .get_mut(na)
                .expect("validated")
                .attach_timed_uart(ua, pa)
                .with_context(|| format!("node '{na}' {ua}"))?;
            self.machines
                .get_mut(nb)
                .expect("validated")
                .attach_timed_uart(ub, pb)
                .with_context(|| format!("node '{nb}' {ub}"))?;
        }
        let links = pairs.len();
        let mut events = Vec::with_capacity(cfg.events.len());
        for (i, e) in cfg.events.iter().enumerate() {
            for link in [e.slow_link, e.cut_link, e.restore_link]
                .into_iter()
                .flatten()
            {
                if link as usize >= links {
                    anyhow::bail!("events[{i}]: no link {link} (the network has {links})");
                }
            }
            if let Some(node) = &e.reset_node {
                if !self.machines.contains_key(node) {
                    anyhow::bail!("events[{i}]: unknown node '{node}'");
                }
            }
            events.push((us_to_ps(e.at_us), e.clone()));
        }
        events.sort_by_key(|(t, _)| *t);
        let mut by_node: std::collections::BTreeMap<String, Vec<(String, u8, String)>> =
            Default::default();
        for m in &cfg.markers {
            if !self.machines.contains_key(&m.node) {
                anyhow::bail!("marker: unknown node '{}'", m.node);
            }
            let name = m
                .name
                .clone()
                .unwrap_or_else(|| format!("{}.{}", m.peripheral, m.pin));
            by_node
                .entry(m.node.clone())
                .or_default()
                .push((m.peripheral.clone(), m.pin, name));
        }
        let mut markers = Vec::new();
        for (node, pins) in by_node {
            let watch: Vec<(String, u8)> = pins.iter().map(|(p, n, _)| (p.clone(), *n)).collect();
            // Watched once every interconnect is built, together with any net
            // pads of the node (markers come first in the watch set).
            self.marker_pins.insert(node.clone(), watch);
            markers.push((node, pins.into_iter().map(|(_, _, n)| n).collect(), 0));
        }
        self.uart_net = Some(WorldUartNet {
            net,
            events,
            next_event: 0,
            markers,
            max_quantum_ps: cfg
                .max_quantum_us
                .map(us_to_ps)
                .unwrap_or(UART_NET_DEFAULT_QUANTUM_PS),
        });
        Ok(())
    }

    /// Watch the pads the interconnects asked for (markers and net pads
    /// share one watch set per node) and build the GPIO nets.
    fn finish_pad_watches(&mut self) -> anyhow::Result<()> {
        use anyhow::Context;
        let configs = std::mem::take(&mut self.pending_nets);
        if !configs.is_empty() && self.ble.is_some() {
            anyhow::bail!("a world cannot have both a BLE air and gpio_net yet");
        }
        match WorldGpio::build(
            &mut self.machines,
            &self.node_hz,
            &self.marker_pins,
            &configs,
        )? {
            Some(gpio) => self.gpio = Some(gpio),
            None => {
                for (node, pins) in &self.marker_pins {
                    self.machines
                        .get_mut(node)
                        .expect("validated")
                        .watch_marker_pins(pins)
                        .with_context(|| format!("marker on node '{node}'"))?;
                }
            }
        }
        Ok(())
    }

    pub fn reset_all(&mut self) -> HashMap<String, SimResult<()>> {
        let mut results = HashMap::new();
        let mut ids: Vec<_> = self.machines.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let result = self
                .machines
                .get_mut(&id)
                .expect("machine id was collected from this world")
                .reset();
            results.insert(id, result);
        }
        results
    }

    /// Build a multi-node environment from an `EnvironmentManifest`.
    ///
    /// Each node is built by [`crate::system::node::build_node`], the same
    /// factory a single-chip run uses, so a node's architecture and boot path
    /// follow from its own chip descriptor and firmware file — Cortex-M and
    /// RISC-V nodes (including ESP32-C3 flash images booted through the genuine
    /// mask ROM) can appear in the same world. Each `uart_cross_link` interconnect wires two nodes' named UARTs
    /// via a [`crate::network::VirtualWireBus`] endpoint pair (point-to-point, the IO-Link
    /// C/Q wire). Paths in the manifest are resolved relative to `root_dir`
    /// (the directory containing the env manifest).
    pub fn from_manifest(
        manifest: labwired_config::EnvironmentManifest,
        root_dir: &std::path::Path,
    ) -> anyhow::Result<Self> {
        Self::from_manifest_with_plugins(manifest, root_dir, &[])
    }

    /// [`Self::from_manifest`] with out-of-tree chip plugins. A node whose
    /// `chip:` spec does not resolve to a descriptor file is offered to the
    /// plugins' embedded YAMLs (matched by the bare spec string) before the
    /// build fails, and each node's bus offers its peripheral types to the
    /// plugins before the in-tree factories.
    pub fn from_manifest_with_plugins(
        manifest: labwired_config::EnvironmentManifest,
        root_dir: &std::path::Path,
        plugins: &[&dyn crate::plugin::ChipPlugin],
    ) -> anyhow::Result<Self> {
        use anyhow::Context;

        let mut resolved = Vec::with_capacity(manifest.nodes.len());
        for node in &manifest.nodes {
            let sys_path = root_dir.join(&node.system);
            let sysman = labwired_config::SystemManifest::from_file(&sys_path)
                .with_context(|| format!("node '{}': system {:?}", node.id, sys_path))?;
            let chip_path = sys_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(&sysman.chip);
            let chip = match labwired_config::ChipDescriptor::from_file(&chip_path) {
                Ok(chip) => chip,
                Err(file_err) => match plugins.iter().find_map(|p| p.chip_yaml(&sysman.chip)) {
                    Some(yaml) => serde_yaml::from_str::<labwired_config::ChipDescriptor>(yaml)
                        .with_context(|| {
                            format!("node '{}': plugin chip '{}'", node.id, sysman.chip)
                        })?,
                    None => {
                        return Err(file_err)
                            .with_context(|| format!("node '{}': chip {:?}", node.id, chip_path));
                    }
                },
            };
            let fw_path = root_dir.join(&node.firmware);
            let firmware = crate::system::node::NodeFirmware::from_file(&fw_path)
                .with_context(|| format!("node '{}': firmware {:?}", node.id, fw_path))?;
            resolved.push(ResolvedWorldNode {
                id: node.id.clone(),
                system: sysman,
                chip,
                firmware,
            });
        }
        Self::from_resolved_with_plugins(manifest, resolved, plugins)
    }

    /// Build a world from artifacts already resolved by the caller. This is
    /// the browser-safe counterpart to [`Self::from_manifest`]: it performs the
    /// same manifest validation, node construction, and interconnect wiring but
    /// never reads a path from the host filesystem.
    pub fn from_resolved(
        manifest: labwired_config::EnvironmentManifest,
        nodes: Vec<ResolvedWorldNode>,
    ) -> anyhow::Result<Self> {
        Self::from_resolved_with_plugins(manifest, nodes, &[])
    }

    fn from_resolved_with_plugins(
        manifest: labwired_config::EnvironmentManifest,
        nodes: Vec<ResolvedWorldNode>,
        plugins: &[&dyn crate::plugin::ChipPlugin],
    ) -> anyhow::Result<Self> {
        use anyhow::Context;

        manifest
            .validate()
            .context("invalid environment manifest")?;
        let expected: std::collections::HashSet<_> =
            manifest.nodes.iter().map(|node| node.id.as_str()).collect();
        let actual: std::collections::HashSet<_> =
            nodes.iter().map(|node| node.id.as_str()).collect();
        if expected != actual || nodes.len() != manifest.nodes.len() {
            anyhow::bail!("resolved node ids must match environment manifest nodes exactly");
        }
        // A world advances its nodes through `step_all`, which runs no
        // co-simulation session. A node's `cosim_models` would therefore never
        // step while the run reported a result, so refuse to build instead.
        // Both the CLI environment runner (`from_manifest`) and the browser
        // `WasmWorld` (`from_resolved`) come through here.
        if let Some(node) = nodes
            .iter()
            .find(|node| !node.system.cosim_models.is_empty())
        {
            anyhow::bail!(
                "co-simulation models are not supported in multi-node worlds yet; node '{}' declares {}",
                node.id,
                node.system.cosim_models.len()
            );
        }

        let mut world = World::new(manifest.name.clone());
        world.rf_medium = build_world_rf_medium(manifest.rf.as_ref());
        // One fab per world: its dice are numbered in manifest order, so the
        // same world built twice gets the same device addresses (and the first
        // two C3s get ...:04 and ...:05, as they did from the process-wide fab
        // in a fresh process).
        let fab = crate::system::efuse::FactoryMacAllocator::new();
        for node in nodes {
            let mut machine = crate::system::node::build_node_in_fab(
                &node.id,
                &node.chip,
                &node.system,
                node.firmware,
                plugins,
                Some(&fab),
            )?;
            // Label each node's UART console with its id so the shared stdout
            // stays readable (line-buffered per node instead of byte-interleaved
            // across all nodes).
            machine.set_stdout_prefix(&format!("[{}] ", node.id));
            world.set_node_hz(&node.id, node.system.cpu_hz.unwrap_or(node.chip.cpu_hz));
            world.add_machine(node.id, machine);
        }

        // One shared lab air for all nodes that carry cellular (and rebind nRF/BLE
        // airs too). Replaces each bus's private from_config air so two UEs share
        // SimMqttFabric fan-out and a single path-loss medium when rf: is set.
        {
            use crate::network::SimMqttFabric;
            use crate::peripherals::ble_air::BleAirBus;
            use crate::peripherals::nrf52::radio::VirtualAirBus;
            use crate::peripherals::rf_medium::{PathLossParams, RfMedium};
            let any_cellular = world.machines.values().any(|m| m.has_cellular_modem());
            if any_cellular {
                let nrf = VirtualAirBus::new();
                if let Some(shared) = &world.rf_medium {
                    if let Ok(guard) = shared.lock() {
                        nrf.attach_medium(
                            RfMedium::new(guard.run_seed()).with_params(guard.params()),
                        );
                    }
                } else {
                    nrf.attach_medium(RfMedium::new(1).with_params(PathLossParams::default()));
                }
                if let Some(rf) = manifest.rf.as_ref() {
                    use crate::peripherals::rf_medium::NodePosition;
                    for (id, pos) in &rf.nodes {
                        nrf.set_node_position(id.clone(), NodePosition { x: pos.x, y: pos.y });
                    }
                }
                let ble = BleAirBus::new();
                let fabric = SimMqttFabric::new();
                for (id, machine) in world.machines.iter_mut() {
                    machine.attach_lab_air(id.as_str(), nrf.clone(), ble.clone(), fabric.clone());
                }
            }
        }

        for ic in &manifest.interconnects {
            match ic.r#type.as_str() {
                "uart_cross_link" => {
                    if ic.nodes.len() != 2 || ic.nodes[0] == ic.nodes[1] {
                        anyhow::bail!("uart_cross_link: requires exactly two unique nodes");
                    }
                    let a = &ic.nodes[0];
                    let b = &ic.nodes[1];
                    if !world.machines.contains_key(a) {
                        anyhow::bail!("uart_cross_link: unknown node '{a}'");
                    }
                    if !world.machines.contains_key(b) {
                        anyhow::bail!("uart_cross_link: unknown node '{b}'");
                    }
                    let a_uart = ic
                        .config
                        .get("node_a_uart")
                        .and_then(|v| v.as_str())
                        .unwrap_or("uart2");
                    let b_uart = ic
                        .config
                        .get("node_b_uart")
                        .and_then(|v| v.as_str())
                        .unwrap_or("uart2");
                    // Links are numbered in manifest order and carried on the
                    // world's one shared medium — the same `VirtualWireBus` the
                    // browser uses, so a link behaves identically on either host.
                    // It needs no tick, so it is not an `Interconnect`.
                    let link_id = world.next_uart_link_id;
                    world.next_uart_link_id += 1;
                    let ea = world.uart_wires.endpoint(link_id, 0);
                    let eb = world.uart_wires.endpoint(link_id, 1);
                    world
                        .machines
                        .get_mut(a)
                        .with_context(|| format!("uart_cross_link: unknown node '{a}'"))?
                        .attach_uart_stream(a_uart, Box::new(ea))?;
                    world
                        .machines
                        .get_mut(b)
                        .with_context(|| format!("uart_cross_link: unknown node '{b}'"))?
                        .attach_uart_stream(b_uart, Box::new(eb))?;
                    world.uart_links.push(UartLink {
                        id: link_id,
                        node_a: a.clone(),
                        node_b: b.clone(),
                    });
                }
                "can_bus" => {
                    let legacy_peripheral = ic
                        .config
                        .get("peripheral")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned);
                    let endpoints = ic
                        .config
                        .get("endpoints")
                        .and_then(|value| value.as_mapping());
                    if legacy_peripheral.is_none() && endpoints.is_none() {
                        anyhow::bail!("can_bus: missing nonblank config.peripheral");
                    }
                    // A manifest's membership order must not alter the behavior
                    // of an otherwise identical topology. CanBus drains attached
                    // endpoints in this order, so use the same lexical ordering
                    // as World::step_all for validation and attachment.
                    let mut node_ids = ic.nodes.clone();
                    node_ids.sort();
                    if node_ids.len() < 2 || node_ids.windows(2).any(|nodes| nodes[0] == nodes[1]) {
                        anyhow::bail!("can_bus: requires at least two unique nodes");
                    }
                    for node_id in &node_ids {
                        if !world.machines.contains_key(node_id) {
                            anyhow::bail!("can_bus: unknown node '{node_id}'");
                        }
                    }

                    if let Some(endpoints) = endpoints {
                        for key in endpoints.keys() {
                            let Some(key) = key.as_str() else {
                                anyhow::bail!("can_bus: endpoint node ids must be strings");
                            };
                            if !node_ids.iter().any(|node| node == key) {
                                anyhow::bail!(
                                    "can_bus: endpoint map contains unknown node '{key}'"
                                );
                            }
                        }
                        if endpoints.len() != node_ids.len() {
                            anyhow::bail!("can_bus: endpoint map must contain every member node");
                        }
                    }

                    let mut can_bus = crate::network::CanBus::new();
                    for node_id in &node_ids {
                        let endpoint = if let Some(endpoints) = endpoints {
                            endpoints
                                .get(serde_yaml::Value::String(node_id.clone()))
                                .and_then(|value| value.as_str())
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .with_context(|| {
                                    format!(
                                        "can_bus: missing nonblank endpoint for node '{node_id}'"
                                    )
                                })?
                        } else {
                            legacy_peripheral
                                .as_deref()
                                .expect("CAN config source was validated above")
                        };
                        let (tx, rx) = can_bus.attach();
                        world
                            .machines
                            .get_mut(node_id)
                            .expect("all can_bus nodes were validated above")
                            .attach_can_bus(endpoint, tx, rx)
                            .with_context(|| format!("can_bus node '{node_id}'"))?;
                    }
                    world.add_interconnect(Box::new(can_bus));
                }
                "egress" => {
                    if ic.nodes.len() != 1 {
                        anyhow::bail!("egress: requires exactly one node");
                    }
                    if !world.machines.contains_key(&ic.nodes[0]) {
                        anyhow::bail!("egress: unknown node '{}'", ic.nodes[0]);
                    }
                    let (node, uart, tx, bus) = build_egress(ic)?;
                    world
                        .machines
                        .get_mut(&node)
                        .with_context(|| format!("egress: unknown node '{node}'"))?
                        .attach_uart_stream(
                            &uart,
                            Box::new(crate::network::egress::tap::EgressTap::new(tx)),
                        )?;
                    world.add_interconnect(Box::new(bus));
                }
                "uart_network" => {
                    world.build_uart_network(ic).context("uart_network")?;
                }
                "gpio_net" => {
                    let cfg = labwired_config::GpioNetConfig::from_interconnect_config(&ic.config)
                        .context("gpio_net")?;
                    for node in &ic.nodes {
                        if !world.machines.contains_key(node) {
                            anyhow::bail!("gpio_net: unknown node '{node}'");
                        }
                    }
                    world.pending_nets.push(cfg);
                }
                "ble_air" => {
                    world.attach_ble_nodes(&ic.nodes).context("ble_air")?;
                }
                "ble_central" => {
                    world.attach_ble_nodes(&ic.nodes).context("ble_central")?;
                    let n = world.ble_central_reports().len();
                    let id = ic
                        .config
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("phone{n}"));
                    let mut map = serde_yaml::Mapping::new();
                    for (k, v) in &ic.config {
                        if k != "id" {
                            map.insert(serde_yaml::Value::String(k.clone()), v.clone());
                        }
                    }
                    // Through JSON: serde_yaml 0.9 only reads `!tagged` enums,
                    // and a script step is written `- read: <uuid>`.
                    let json = serde_json::to_value(serde_yaml::Value::Mapping(map))
                        .with_context(|| format!("ble_central '{id}': config"))?;
                    let cfg: crate::peripherals::ble_central::CentralConfig =
                        serde_json::from_value(json)
                            .with_context(|| format!("ble_central '{id}': config"))?;
                    world.add_ble_central(id, cfg)?;
                }
                other => anyhow::bail!("unsupported interconnect type '{other}'"),
            }
        }
        world.finish_pad_watches().context("gpio_net")?;

        Ok(world)
    }
}

/// Microseconds (manifest unit) to picoseconds.
fn us_to_ps(us: f64) -> u64 {
    (us * 1_000_000.0).round().max(0.0) as u64
}

/// Build a shared [`crate::peripherals::rf_medium::RfMedium`] from env `rf:`.
fn build_world_rf_medium(
    rf: Option<&labwired_config::EnvironmentRfConfig>,
) -> Option<std::sync::Arc<std::sync::Mutex<crate::peripherals::rf_medium::RfMedium>>> {
    let rf = rf?;
    use crate::peripherals::rf_medium::{NodePosition, PathLossParams, RfMedium};
    let mut params = PathLossParams::default();
    if let Some(floor) = rf.rssi_floor_dbm {
        params.rssi_floor_dbm = floor;
    }
    if let Some(exp) = rf.path_loss_exponent {
        params.exponent = exp;
    }
    if let Some(r) = rf.ref_loss_db {
        params.ref_loss_db = r;
    }
    let mut medium = RfMedium::new(rf.seed).with_params(params);
    for (id, pos) in &rf.nodes {
        medium.set_node(id.clone(), NodePosition { x: pos.x, y: pos.y });
    }
    Some(std::sync::Arc::new(std::sync::Mutex::new(medium)))
}

/// Build the egress tap channel and `EgressBus` for an `egress` interconnect.
/// Returns `(node_id, uart_id, tap_sender, bus)`. Transports connect lazily on
/// first send, so this never blocks on the network.
#[allow(clippy::type_complexity)]
fn build_egress(
    ic: &labwired_config::InterconnectConfig,
) -> anyhow::Result<(
    String,
    String,
    std::sync::mpsc::Sender<crate::network::egress::EgressItem>,
    crate::network::egress::bus::EgressBus,
)> {
    use crate::network::egress::bus::EgressBus;
    use crate::network::egress::transport::{EgressTransport, HttpPoster, MqttPublisher, TcpSink};
    use crate::network::egress::{BufferPolicy, EgressItem, EncodingKind};
    use anyhow::Context;

    let node = ic
        .nodes
        .first()
        .context("egress needs exactly one node")?
        .clone();
    let get = |k: &str| ic.config.get(k).and_then(|v| v.as_str());
    let uart = get("uart").unwrap_or("usart2").to_string();
    let encoding = match get("encoding").unwrap_or("raw") {
        "raw" => EncodingKind::Raw,
        "ndjson-trace" => EncodingKind::NdjsonTrace,
        "frames-json" => EncodingKind::FramesJson,
        other => anyhow::bail!("egress: unknown encoding '{other}'"),
    };
    let url = get("url").context("egress: missing 'url'")?.to_string();
    let transport: Box<dyn EgressTransport> = match get("transport").unwrap_or("tcp") {
        "tcp" => Box::new(TcpSink::new(url)),
        "mqtt" => {
            let (host, port) = parse_mqtt_url(&url)?;
            let topic = get("topic")
                .context("egress: mqtt needs 'topic'")?
                .to_string();
            Box::new(MqttPublisher::lazy(host, port, topic))
        }
        "http" => Box::new(HttpPoster::new(url)?),
        other => anyhow::bail!("egress: unknown transport '{other}'"),
    };
    let policy = BufferPolicy {
        max: ic
            .config
            .get("buffer_max")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(BufferPolicy::default().max),
    };
    let (tx, rx) = std::sync::mpsc::channel::<EgressItem>();
    let bus = EgressBus::new(rx, encoding, policy, transport);
    Ok((node, uart, tx, bus))
}

/// Parse `mqtt://host:port` → (host, port).
fn parse_mqtt_url(url: &str) -> anyhow::Result<(String, u16)> {
    let rest = url.strip_prefix("mqtt://").unwrap_or(url);
    let (host, port) = rest
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("mqtt url needs host:port: {url}"))?;
    Ok((host.to_string(), port.parse()?))
}

#[cfg(test)]
mod egress_manifest_tests {
    use super::*;
    use labwired_config::InterconnectConfig;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> InterconnectConfig {
        let mut config = HashMap::new();
        for (k, v) in pairs {
            config.insert(k.to_string(), serde_yaml::Value::String(v.to_string()));
        }
        InterconnectConfig {
            r#type: "egress".to_string(),
            nodes: vec!["sensor_node".to_string()],
            config,
        }
    }

    #[test]
    fn parses_tcp_egress_config() {
        let c = cfg(&[
            ("uart", "usart2"),
            ("transport", "tcp"),
            ("url", "127.0.0.1:9"),
            ("encoding", "raw"),
        ]);
        let (node, uart, _tx, _bus) = build_egress(&c).unwrap();
        assert_eq!(node, "sensor_node");
        assert_eq!(uart, "usart2");
    }

    #[test]
    fn rejects_unknown_transport() {
        let c = cfg(&[
            ("uart", "usart2"),
            ("transport", "carrier-pigeon"),
            ("url", "x"),
        ]);
        assert!(build_egress(&c).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::cpu::cortex_m::CortexM;

    #[test]
    fn test_multi_node_basic_sync() {
        let mut world = World::new("test-world".to_string());

        let bus1 = SystemBus::new();
        let cpu1 = CortexM::new();
        let machine1 = Machine::new(cpu1, bus1);

        let bus2 = SystemBus::new();
        let cpu2 = CortexM::new();
        let machine2 = Machine::new(cpu2, bus2);

        world.add_machine("node1".to_string(), Box::new(machine1));
        world.add_machine("node2".to_string(), Box::new(machine2));

        // Step the world
        let results = world.step_all();
        assert_eq!(results.len(), 2);
        assert!(results.get("node1").unwrap().is_ok());
        assert!(results.get("node2").unwrap().is_ok());

        assert_eq!(world.machines.get("node1").unwrap().total_cycles(), 1);
        assert_eq!(world.machines.get("node2").unwrap().total_cycles(), 1);
    }

    use crate::network::CanBus;
    use crate::peripherals::can::CanController;
    use crate::Peripheral;

    #[test]
    fn test_can_bus_transmission() {
        let mut world = World::new("test-can".to_string());

        let mut can_bus = CanBus::new();
        let (tx1, rx1) = can_bus.attach();
        let (tx2, rx2) = can_bus.attach();

        world.add_interconnect(Box::new(can_bus));

        let mut can1 = CanController::new(tx1, rx1);
        let mut can2 = CanController::new(tx2, rx2);

        can1.write(0x00, 0xAA).unwrap();
        can1.write(0x04, 0x12).unwrap();
        can1.write(0x05, 0x34).unwrap();
        can1.write(0x08, 0x01).unwrap();

        let _ = world.step_all();

        let _ = can2.tick();

        let status = can2.read(0x08).unwrap();
        assert_eq!(status, 1, "RX pending should be 1");

        let rx_id = can2.read(0x0C).unwrap();
        assert_eq!(rx_id, 0xAA);

        let rx_data_0 = can2.read(0x10).unwrap();
        let rx_data_1 = can2.read(0x11).unwrap();
        assert_eq!(rx_data_0, 0x12);
        assert_eq!(rx_data_1, 0x34);
    }

    use crate::network::WirelessBus;
    use crate::peripherals::radio::RadioController;

    #[test]
    fn test_wireless_bus_transmission() {
        let mut world = World::new("test-wireless".to_string());

        let mut wireless_bus = WirelessBus::new();
        let (tx1, rx1) = wireless_bus.attach();
        let (tx2, rx2) = wireless_bus.attach();

        world.add_interconnect(Box::new(wireless_bus));

        let mut radio1 = RadioController::new(tx1, rx1);
        let mut radio2 = RadioController::new(tx2, rx2);

        // Setup channels (Channel 10)
        radio1.write(0x00, 10).unwrap(); // TX CH
        radio2.write(0x00, 10).unwrap(); // Also needs to be on index 10 to receive

        // Trigger TX on radio1
        radio1.write(0x08, 0x01).unwrap();

        // Step the world
        let _ = world.step_all();

        // Tick radio2 to process incoming packet
        let _ = radio2.tick();

        let status = radio2.read(0x0C).unwrap();
        assert_eq!(status, 1, "RX pending should be 1");

        let rx_ch = radio2.read(0x10).unwrap();
        assert_eq!(rx_ch, 10);
    }
}
