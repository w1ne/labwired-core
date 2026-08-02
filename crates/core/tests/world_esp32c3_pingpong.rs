// Two ESP32-C3 nodes exchanging bytes over a modelled UART1 wire.
//
// This is the end-to-end proof for the multi-chip work, and it deliberately
// uses the ESP32 family rather than the Cortex-M IO-Link station, because that
// family is where every seam used to break:
//
//   * `World` refused non-Cortex-M nodes outright (fixed by the node factory).
//   * `attach_uart_stream_by_id` downcast to the concrete `Uart`, so a C3's
//     `uart1` reported "is not a UART" and could not be cross-linked at all.
//   * `EspUart` had no stream support, so even a bound peer would neither be
//     polled for RX nor see TX.
//   * every node UART shared one capture buffer, so the link's octets spliced
//     into the console text (`sceurvveern  up`) and no assertion could be
//     trusted.
//
// The firmware is bare-metal on purpose — it pokes the C3's UART registers
// directly, so a regression in the register model shows up here rather than
// being absorbed by a HAL. See examples/ci-two-c3-link/README.md.

use labwired_config::EnvironmentManifest;
use labwired_core::world::World;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn fixture_root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/ci-two-c3-link"
    ))
    .to_path_buf()
}

/// Console text captured per node. The link UART must NOT appear here — that is
/// half of what this test checks.
fn run_world(steps: usize) -> Vec<(String, String)> {
    let root = fixture_root();
    let env = EnvironmentManifest::from_file(root.join("env.yaml")).expect("parse env.yaml");
    let mut world = World::from_manifest(env, &root).expect("build two-C3 world");
    assert_eq!(world.machines.len(), 2, "two C3 nodes expected");

    let mut ids: Vec<String> = world.machines.keys().cloned().collect();
    ids.sort();

    let mut sinks: Vec<(String, Arc<Mutex<Vec<u8>>>)> = Vec::new();
    for id in &ids {
        let sink = Arc::new(Mutex::new(Vec::new()));
        world
            .machines
            .get_mut(id)
            .expect("id came from the world")
            // `false` = capture only, no host-console echo.
            .attach_uart_tx_sink(sink.clone(), false)
            .expect("attach console capture");
        sinks.push((id.clone(), sink));
    }

    for _ in 0..steps {
        let results = world.step_all();
        assert!(
            results.values().all(|r| r.is_ok()),
            "a node failed to step: {results:?}"
        );
    }

    sinks
        .into_iter()
        .map(|(id, sink)| {
            let bytes = sink.lock().expect("sink not poisoned").clone();
            (id, String::from_utf8_lossy(&bytes).into_owned())
        })
        .collect()
}

#[test]
fn two_esp32c3_nodes_rally_over_a_cross_linked_uart1() {
    // One world run answers both questions below, so the step budget is paid
    // once rather than per assertion.
    let consoles = run_world(20_000_000);
    let get = |name: &str| {
        consoles
            .iter()
            .find(|(id, _)| id == name)
            .map(|(_, text)| text.clone())
            .unwrap_or_else(|| panic!("no console for node '{name}'"))
    };

    let server = get("server");
    let client = get("client");

    // The server only prints a rally once it has read a full "PONG\n" back off
    // the wire, so three rallies means three complete round trips through the
    // peer's firmware — not merely that bytes were queued.
    assert!(
        server.contains("rally 1"),
        "no completed round trip; server console was:\n{server}"
    );
    assert!(
        server.contains("rally 3") && server.contains("server done"),
        "did not reach three round trips; server console was:\n{server}"
    );
    assert!(
        !server.contains("no PONG"),
        "server timed out waiting for its peer:\n{server}"
    );
    assert_eq!(
        client.matches("client: returned").count(),
        3,
        "peer did not answer exactly three times:\n{client}"
    );

    // Regression guard for the interleaving bug: uart0 (console) and uart1
    // (link) both pushed into one per-node buffer, so consoles came out spliced
    // and PING/PONG octets landed in the middle of assertion text.
    for (id, text) in &consoles {
        assert!(
            !text.contains("PING\n") && !text.contains("PONG\n"),
            "raw link octets leaked into node '{id}' console:\n{text}"
        );
        assert!(
            text.contains(&format!("{id} up")),
            "node '{id}' console banner is garbled:\n{text}"
        );
    }
}
