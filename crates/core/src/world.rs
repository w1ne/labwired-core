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
}

/// Type-erased trait for machines to allow heterogeneous machines in the world.
pub trait MachineTrait: Send {
    fn name(&self) -> &str;
    fn step(&mut self) -> SimResult<()>;
    fn reset(&mut self) -> SimResult<()>;
    fn total_cycles(&self) -> u64;
    fn read_u8(&self, addr: u64) -> SimResult<u8>;
    fn write_u8(&mut self, addr: u64, val: u8) -> SimResult<()>;
    /// Attach a UART stream device (e.g. a `UartCrossLink` wire endpoint) to a
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
        }
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
    /// Each node is a Cortex-M `Machine` built from its `SystemManifest` + chip,
    /// with its firmware ELF loaded and the CPU reset to boot from the vector
    /// table. Each `uart_cross_link` interconnect wires two nodes' named UARTs
    /// via a [`crate::network::UartCrossLink`] (point-to-point, the IO-Link
    /// C/Q wire). Paths in the manifest are resolved relative to `root_dir`
    /// (the directory containing the env manifest).
    pub fn from_manifest(
        manifest: labwired_config::EnvironmentManifest,
        root_dir: &std::path::Path,
    ) -> anyhow::Result<Self> {
        use anyhow::Context;

        manifest
            .validate()
            .context("invalid environment manifest")?;
        let mut world = World::new(manifest.name.clone());

        for node in &manifest.nodes {
            let sys_path = root_dir.join(&node.system);
            let sysman = labwired_config::SystemManifest::from_file(&sys_path)
                .with_context(|| format!("node '{}': system {:?}", node.id, sys_path))?;
            let chip_path = sys_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(&sysman.chip);
            let chip = labwired_config::ChipDescriptor::from_file(&chip_path)
                .with_context(|| format!("node '{}': chip {:?}", node.id, chip_path))?;
            if !is_cortex_m_chip(&chip) {
                anyhow::bail!(
                    "node '{}': environment worlds currently support only Cortex-M nodes; each node requires an explicit Cortex-M core (`chip.arch: arm`, `chip.core: cortex-m*`). chip '{}' has architecture {:?} and core {:?}",
                    node.id,
                    chip.name,
                    chip.arch,
                    chip.core
                );
            }
            let fw_path = root_dir.join(&node.firmware);
            let image = load_elf_image(&fw_path)
                .with_context(|| format!("node '{}': firmware {:?}", node.id, fw_path))?;
            validate_cortex_m_firmware(&node.id, &chip, &image)?;
            let mut bus = crate::bus::SystemBus::from_config(&chip, &sysman)
                .with_context(|| format!("node '{}': build bus", node.id))?;
            let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
            let mut machine = Machine::new(cpu, bus);
            machine
                .load_firmware(&image)
                .map_err(|e| anyhow::anyhow!("node '{}': load firmware: {e:?}", node.id))?;
            machine
                .reset()
                .map_err(|e| anyhow::anyhow!("node '{}': reset: {e:?}", node.id))?;
            // Label each node's UART console with its id so the shared stdout
            // stays readable (line-buffered per node instead of byte-interleaved
            // across all nodes).
            let prefix = format!("[{}] ", node.id);
            for p in machine.bus.peripherals.iter_mut() {
                if let Some(uart) = p
                    .dev
                    .as_any_mut()
                    .and_then(|any| any.downcast_mut::<crate::peripherals::uart::Uart>())
                {
                    uart.set_stdout_prefix(prefix.clone());
                }
            }
            world.add_machine(node.id.clone(), Box::new(machine));
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
                    let (link, ea, eb) = crate::network::UartCrossLink::new(a.clone(), b.clone());
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
                    world.add_interconnect(Box::new(link));
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

fn is_cortex_m_chip(chip: &labwired_config::ChipDescriptor) -> bool {
    chip.arch == labwired_config::Arch::Arm
        && chip
            .core
            .as_deref()
            .is_some_and(|core| core.trim().to_ascii_lowercase().starts_with("cortex-m"))
}

fn validate_cortex_m_firmware(
    node_id: &str,
    chip: &labwired_config::ChipDescriptor,
    image: &crate::memory::ProgramImage,
) -> anyhow::Result<()> {
    use anyhow::Context;

    if image.arch != crate::Arch::Arm {
        anyhow::bail!(
            "node '{}': firmware architecture {:?} is incompatible with Cortex-M system chip '{}'; environment worlds require an ARM ELF with a valid Cortex-M Thumb reset vector",
            node_id,
            image.arch,
            chip.name
        );
    }

    let flash_size = labwired_config::parse_size(&chip.flash.size).with_context(|| {
        format!(
            "node '{}': invalid flash size for chip '{}'",
            node_id, chip.name
        )
    })?;
    let ram_size = labwired_config::parse_size(&chip.ram.size).with_context(|| {
        format!(
            "node '{}': invalid RAM size for chip '{}'",
            node_id, chip.name
        )
    })?;
    let vector_base = chip
        .flash
        .base
        .checked_add(chip.reset_vector_offset)
        .context("Cortex-M reset vector address overflow")?;
    let stack_pointer = image_u32_at(image, vector_base);
    let reset_handler = image_u32_at(image, vector_base.saturating_add(4));
    let reset_target = reset_handler.map(|handler| u64::from(handler & !1));
    let valid_stack = stack_pointer.is_some_and(|stack| {
        let stack = u64::from(stack);
        stack >= chip.ram.base && stack <= chip.ram.base.saturating_add(ram_size)
    });
    let valid_reset = reset_handler.is_some_and(|handler| handler & 1 == 1)
        && reset_target.is_some_and(|target| {
            target >= chip.flash.base && target < chip.flash.base.saturating_add(flash_size)
        });
    if !valid_stack || !valid_reset {
        anyhow::bail!(
            "node '{}': firmware does not contain a valid Cortex-M Thumb reset vector for chip '{}'",
            node_id,
            chip.name
        );
    }

    Ok(())
}

fn image_u32_at(image: &crate::memory::ProgramImage, address: u64) -> Option<u32> {
    let mut bytes = [0_u8; 4];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let byte_address = address.checked_add(index as u64)?;
        *byte = image.segments.iter().find_map(|segment| {
            let offset = usize::try_from(byte_address.checked_sub(segment.start_addr)?).ok()?;
            segment.data.get(offset).copied()
        })?;
    }
    Some(u32::from_le_bytes(bytes))
}

/// Parse an ELF file into a `ProgramImage` using goblin (core cannot depend on
/// the `loader` crate — it depends on core). PT_LOAD segments are placed at
/// their load address (`p_paddr`), matching how Cortex-M flash images and the
/// `.data` LMA-in-flash convention work.
fn load_elf_image(path: &std::path::Path) -> anyhow::Result<crate::memory::ProgramImage> {
    use anyhow::Context;
    use goblin::elf::program_header::PT_LOAD;
    use goblin::elf::Elf;

    let bytes = std::fs::read(path).with_context(|| format!("read ELF {path:?}"))?;
    let elf = Elf::parse(&bytes).with_context(|| format!("parse ELF {path:?}"))?;
    let arch = match elf.header.e_machine {
        goblin::elf::header::EM_ARM => crate::Arch::Arm,
        goblin::elf::header::EM_RISCV => crate::Arch::RiscV,
        machine => anyhow::bail!(
            "unsupported ELF machine type {machine} in {path:?}; environment worlds support Arm firmware only"
        ),
    };
    let mut image = crate::memory::ProgramImage::new(elf.entry, arch);
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let off = ph.p_offset as usize;
        let n = ph.p_filesz as usize;
        if off + n <= bytes.len() {
            image.add_segment(ph.p_paddr, bytes[off..off + n].to_vec());
        }
    }
    Ok(image)
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
