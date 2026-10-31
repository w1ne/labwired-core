// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Three CC2538DK nodes run the unmodified Contiki-NG v4.9 `examples/rpl-udp`
//! over one shared IEEE 802.15.4 air: an RPL root running `udp-server` and
//! two `udp-client` nodes, the topology of Renode's
//! `tests/platforms/CC2538/cc2538_rpl-udp.robot` (and of Contiki-NG's own
//! `examples/rpl-udp/rpl-udp.robot`). Each client must join the DODAG, send
//! requests to the root and get every one echoed back, observed only from
//! the UART lines the firmware prints, the way the Robot test observes them.
//!
//! What it exercises end to end: the CC2538 RF core TX through the µDMA into
//! `RFDATA`, the shared air, RX filtering into the RX FIFO, the FIFOP
//! interrupt (IRQ 141) waking the WFI idle loop, RX through the µDMA, the
//! hardware AUTOACK the CSMA MAC waits for, CCA, and the sleep timer driving
//! Contiki's etimers across idle fast-forward.
//!
//! Node identity is the IEEE address in the flash information page
//! (`0x0028_0028`), written per node exactly as the Robot test does
//! (`id, 0, 0, 0, 0, 0, 0, 0` → link-layer address `0000.0000.0000.000<id>`,
//! global address `fd00::200:0:0:<id>`), so both clients run one ELF.
//!
//! Lockstep: the node furthest behind in simulated time runs next, up to
//! [`WINDOW_NS`] past the slowest other node, with idle fast-forward on (the
//! rfcore caps each idle skip to the bus-tick poll quantum while its
//! receiver is on). A frame is pushed at its TX strobe and delivered once a
//! receiver's own clock passes its last bit; the window is shorter than the
//! 192-µs turnaround plus the shortest frame, so no frame is ever delivered
//! late.
//!
//! Needs the event scheduler (idle fast-forward); without it the ~30 s of
//! simulated time is billions of interpreted cycles. `required-features` in
//! Cargo.toml skips the target without it; the PR lane is core-ci.yml's
//! "Scheduler-only fidelity lanes" step.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::ieee802154_air::Ieee802154AirBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::sync::{Arc, Mutex};

/// CC2538DK core clock in the system manifest (16-MHz HF-RC, SYS_DIV 1).
const HZ: u64 = 16_000_000;
/// Lockstep window: a node may run this far past the slowest other node.
const WINDOW_NS: u64 = 250_000;
/// Simulated-time budget. Requests go out every 10 s ± 1 s after a random
/// first delay under 10 s; three round trips per client fit in ~40 s.
const BUDGET_NS: u64 = 60_000_000_000;
const REPEATS: u32 = 3;

/// Flash information page: primary IEEE address (Contiki-NG ieee-addr.h).
const IEEE_ADDR: u64 = 0x0028_0028;

struct Node {
    name: &'static str,
    machine: Machine<CortexM>,
    uart: Arc<Mutex<Vec<u8>>>,
}

impl Node {
    fn new(name: &'static str, elf: &str, id: u8, air: &Ieee802154AirBus) -> Self {
        let chip_path = root("configs/chips/cc2538sf53.yaml");
        let chip = ChipDescriptor::from_file(&chip_path).expect("cc2538sf53 chip");
        let mut manifest =
            SystemManifest::from_file(root("configs/systems/cc2538dk.yaml")).expect("cc2538dk");
        manifest.chip = chip_path.to_str().expect("utf8").to_string();
        let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
        let uart = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart.clone(), false);
        bus.attach_ieee802154_air(air.clone());
        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        machine.config.idle_fast_forward_enabled = true;
        let image = labwired_loader::load_elf(&root(&format!("tests/fixtures/{elf}")))
            .unwrap_or_else(|e| panic!("load {elf}: {e}"));
        machine.load_firmware(&image).expect("load firmware");
        // The Robot test's per-node IEEE address, one byte per word.
        let mut addr = [0u8; 8];
        addr[0] = id;
        for (i, b) in addr.iter().enumerate() {
            machine.bus.write_u8(IEEE_ADDR + i as u64, *b).unwrap();
        }
        Self {
            name,
            machine,
            uart,
        }
    }

    fn time_ns(&self) -> u64 {
        (u128::from(self.machine.total_cycles) * 1_000_000_000 / u128::from(HZ)) as u64
    }

    fn console(&self) -> String {
        String::from_utf8_lossy(&self.uart.lock().unwrap()).into_owned()
    }
}

/// `needle` lines appear in `hay` in order.
fn in_order(hay: &str, needles: &[String]) -> bool {
    let mut rest = hay;
    for n in needles {
        match rest.find(n.as_str()) {
            Some(i) => rest = &rest[i + n.len()..],
            None => return false,
        }
    }
    true
}

fn client_lines(n: u32) -> Vec<String> {
    (0..n)
        .flat_map(|i| {
            [
                format!("Sending request {i} to fd00::200:0:0:1"),
                format!("Received response 'hello {i}' from fd00::200:0:0:1"),
            ]
        })
        .collect()
}

fn server_lines(client: u8, n: u32) -> Vec<String> {
    (0..n)
        .flat_map(|i| {
            [
                format!("Received request 'hello {i}' from fd00::200:0:0:{client}"),
                "Sending response.".to_string(),
            ]
        })
        .collect()
}

fn done(nodes: &[Node]) -> bool {
    let server = nodes[0].console();
    in_order(&server, &server_lines(2, REPEATS))
        && in_order(&server, &server_lines(3, REPEATS))
        && in_order(&nodes[1].console(), &client_lines(REPEATS))
        && in_order(&nodes[2].console(), &client_lines(REPEATS))
}

fn run_world() -> (Vec<Node>, Ieee802154AirBus) {
    let air = Ieee802154AirBus::new();
    let mut nodes = vec![
        Node::new("server", "cc2538dk-contiki-udp-server.elf", 1, &air),
        Node::new("client-1", "cc2538dk-contiki-udp-client.elf", 2, &air),
        Node::new("client-2", "cc2538dk-contiki-udp-client.elf", 3, &air),
    ];
    let window = WINDOW_NS * HZ / 1_000_000_000;
    let mut rounds = 0u64;
    loop {
        let (slow, t) = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (i, n.time_ns()))
            .min_by_key(|&(i, t)| (t, i))
            .unwrap();
        if t >= BUDGET_NS || (rounds.is_multiple_of(4096) && done(&nodes)) {
            break;
        }
        let others = nodes
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != slow)
            .map(|(_, n)| n.machine.total_cycles)
            .min()
            .unwrap();
        let target = others.max(nodes[slow].machine.total_cycles) + window;
        nodes[slow]
            .machine
            .advance_to_cycle(target)
            .unwrap_or_else(|e| panic!("{} faulted: {e:?}", nodes[slow].name));
        rounds += 1;
    }
    (nodes, air)
}

#[test]
fn rpl_udp_server_and_two_clients_talk_over_the_air() {
    let (nodes, air) = run_world();
    for n in &nodes {
        eprintln!(
            "==== {} (t = {} ms)\n{}",
            n.name,
            n.time_ns() / 1_000_000,
            n.console()
        );
    }
    let server = nodes[0].console();
    assert!(
        server.contains("Link-layer address: 0000.0000.0000.0001"),
        "the server's IEEE address comes from the info page"
    );
    assert!(
        in_order(&server, &server_lines(2, REPEATS)),
        "server answers client-1 ({REPEATS} requests)"
    );
    assert!(
        in_order(&server, &server_lines(3, REPEATS)),
        "server answers client-2 ({REPEATS} requests)"
    );
    for n in &nodes[1..] {
        assert!(
            in_order(&n.console(), &client_lines(REPEATS)),
            "{} gets {REPEATS} responses",
            n.name
        );
    }
    // Unicast data went out with the ACK request bit and the hardware
    // AUTOACK answered: at least one 5-byte ACK frame crossed the air.
    assert!(
        air.trace_snapshot()
            .iter()
            .any(|f| f.mpdu.len() == 5 && f.mpdu[0] == 0x02),
        "hardware ACKs on the air"
    );
}
