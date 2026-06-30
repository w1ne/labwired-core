#![cfg(feature = "iolink-native")]

use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::world::{MachineTrait, World};
use labwired_core::Machine;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

struct IolinkNode {
    machine: Box<dyn MachineTrait>,
    uart: Arc<Mutex<Vec<u8>>>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_path(path: &str) -> PathBuf {
    repo_root().join(path)
}

fn build_node(system_path: &Path, firmware_path: &Path) -> IolinkNode {
    let mut bus = labwired_core::system::builder::build_system_bus(Some(system_path))
        .expect("build AL2205 IO-Link system bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    assert!(
        bus.attach_uart_tx_sink_named("uart1", uart.clone(), false),
        "debug UART uart1 must exist in the AL2205 system manifest"
    );

    let program = labwired_loader::load_elf(firmware_path).expect("load AL2205 firmware ELF");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&program)
        .expect("load AL2205 firmware into simulated node");

    IolinkNode {
        machine: Box::new(machine),
        uart,
    }
}

#[test]
fn two_l476_nodes_run_native_iolink_ports_under_world_orchestration() {
    let system_path = fixture_path("examples/al2205-iolink-dido/system.yaml");
    let firmware_path = fixture_path("examples/al2205-iolink-dido/firmware/al2205_dido.elf");
    assert!(
        firmware_path.exists(),
        "AL2205 firmware ELF missing at {}; run `make -C examples/al2205-iolink-dido/firmware` first",
        firmware_path.display()
    );

    let node_a = build_node(&system_path, &firmware_path);
    let node_a_uart = node_a.uart.clone();
    let node_b = build_node(&system_path, &firmware_path);
    let node_b_uart = node_b.uart.clone();

    let mut world = World::new("iolink-two-device-nodes".to_string());
    world.add_machine("al2205-node-a".to_string(), node_a.machine);
    world.add_machine("al2205-node-b".to_string(), node_b.machine);

    let mut reached = None;
    for step in 0..200_000u32 {
        for (id, result) in world.step_all() {
            result.unwrap_or_else(|e| panic!("{id} failed at world step {step}: {e:?}"));
        }

        let a = String::from_utf8_lossy(&node_a_uart.lock().unwrap()).into_owned();
        let b = String::from_utf8_lossy(&node_b_uart.lock().unwrap()).into_owned();
        if has_two_port_operate(&a) && has_two_port_operate(&b) {
            reached = Some(step);
            break;
        }
    }

    let a = String::from_utf8_lossy(&node_a_uart.lock().unwrap()).into_owned();
    let b = String::from_utf8_lossy(&node_b_uart.lock().unwrap()).into_owned();

    assert!(
        reached.is_some(),
        "both nodes should reach IO-Link OPERATE within the world step budget\nnode A UART:\n{a}\nnode B UART:\n{b}"
    );
    assert!(a.contains("AL2205 BOOT"), "node A boot log missing:\n{a}");
    assert!(b.contains("AL2205 BOOT"), "node B boot log missing:\n{b}");
    assert!(
        has_two_port_operate(&a),
        "node A did not operate both ports:\n{a}"
    );
    assert!(
        has_two_port_operate(&b),
        "node B did not operate both ports:\n{b}"
    );
}

fn has_two_port_operate(text: &str) -> bool {
    text.contains("PORT2 STATE=04 OPERATE PD=A5") && text.contains("PORT3 STATE=04 OPERATE PD=3C")
}
