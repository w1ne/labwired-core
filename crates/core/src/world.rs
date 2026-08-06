// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::network::Interconnect;
use crate::{Bus, Cpu, Machine, SimResult};
use std::collections::HashMap;

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
}

/// One point-to-point serial link between two nodes, as carried on the world's
/// [`crate::network::VirtualWireBus`]. `node_a` sits on side 0, `node_b` side 1.
#[derive(Debug, Clone)]
pub struct UartLink {
    pub id: u32,
    pub node_a: String,
    pub node_b: String,
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

    fn attach_uart_tx_sink(
        &mut self,
        sink: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        echo_stdout: bool,
    ) -> anyhow::Result<()> {
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

    fn attach_can_bus(
        &mut self,
        can_id: &str,
        tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
        rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        self.bus.attach_can_bus_by_id(can_id, tx, rx)
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
        }
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

        manifest
            .validate()
            .context("invalid environment manifest")?;
        let mut world = World::new(manifest.name.clone());
        world.rf_medium = build_world_rf_medium(manifest.rf.as_ref());

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
            let mut machine = crate::system::node::build_node_with_plugins(
                &node.id, &chip, &sysman, firmware, plugins,
            )?;
            // Label each node's UART console with its id so the shared stdout
            // stays readable (line-buffered per node instead of byte-interleaved
            // across all nodes).
            machine.set_stdout_prefix(&format!("[{}] ", node.id));
            world.add_machine(node.id.clone(), machine);
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
                    let peripheral = ic
                        .config
                        .get("peripheral")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .context("can_bus: missing nonblank config.peripheral")?;
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

                    let mut can_bus = crate::network::CanBus::new();
                    for node_id in &node_ids {
                        let (tx, rx) = can_bus.attach();
                        world
                            .machines
                            .get_mut(node_id)
                            .expect("all can_bus nodes were validated above")
                            .attach_can_bus(peripheral, tx, rx)
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
                other => anyhow::bail!("unsupported interconnect type '{other}'"),
            }
        }

        Ok(world)
    }
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
