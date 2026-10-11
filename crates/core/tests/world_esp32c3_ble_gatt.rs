//! BLE **connections and GATT** between simulated nodes, on real firmware.
//!
//! Two gates, both through [`World`] — the same construction the CLI
//! environment runner and the browser's `WasmWorld` use:
//!
//! 1. `two_c3_nodes_connect_discover_read_write_and_notify` — two ESP32-C3s,
//!    one running Arduino-ESP32's stock `BLE_notify` example (a GATT server
//!    that notifies a counter), the other the stock `BLE_client` example (a
//!    central that scans, connects, discovers the service, reads, writes and
//!    subscribes). Both run the genuine mask ROM and the genuine RW-BLE link
//!    layer; LabWired models only the baseband core under it
//!    (`peripherals/esp32c3/bt_link.rs`). Everything asserted is what the
//!    sketches print, i.e. what a user sees on the two serial monitors.
//! 2. `a_scripted_phone_connects_to_the_c3_gatt_server` — the same server,
//!    driven by a `ble_central` interconnect (the "phone": connect, discover,
//!    read, write, subscribe, wait for notifications, disconnect), so a user
//!    can test their GATT server without a second MCU.
//!
//! Each runs twice and must produce the same transcript: the world steps its
//! nodes in time lockstep, so the exchange does not depend on host timing.
//!
//! # Firmware
//!
//! Stock examples from framework-arduinoespressif32 (ESP-IDF v4.4.7
//! 38eeba213a, arduino-lib-builder), unmodified: `BLE/examples/BLE_notify`
//! and `BLE/examples/BLE_client`, built for an ESP32-C3 with the hosted
//! PlatformIO toolchain (`labwired_compile`). The composed 4 MiB flash images
//! are NOT committed; their parts are content-addressed and pinned in
//! `scripts/ci/c3-ble-gatt-{notify,client}-flash.sha256`:
//!
//!     scripts/ci/fetch-c3-ble-flash.sh fixtures/esp32c3-ble scripts/ci/c3-ble-gatt-notify-flash.sha256
//!     scripts/ci/fetch-c3-ble-flash.sh fixtures/esp32c3-ble scripts/ci/c3-ble-gatt-client-flash.sha256
//!     cargo test --release -p labwired-core --features event-scheduler \
//!         --test world_esp32c3_ble_gatt -- --ignored --nocapture
//!
//! Absent images skip with a message; `LABWIRED_REQUIRE_C3_BLE=1` turns that
//! into a failure, the contract every C3 BLE gate holds.
//!
//! ⚠ It needs `--features event-scheduler` (the BT model's radio engine runs
//! from `on_event`). `crates/core/Cargo.toml` declares that as the target's
//! `required-features`, so cargo skips the target without it — and a bare
//! `--test world_esp32c3_ble_gatt` is an error rather than an empty green run.

use labwired_config::{ChipDescriptor, EnvironmentManifest, SystemManifest};
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{ResolvedWorldNode, World};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const CHAR_UUID: &str = "beb5483e-36e1-4688-b7f5-ea07361b26a8";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The pinned image digest from a manifest's `image` line.
fn pinned_image_sha(manifest: &str) -> String {
    let path = repo_root().join("scripts/ci").join(manifest);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .find_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 3 && f[1] == "image").then(|| f[0].to_string())
        })
        .unwrap_or_else(|| panic!("{manifest}: no image line"))
}

/// The composed flash image, verified against its pin, or `None` (skip).
fn flash_image(name: &str) -> Option<Vec<u8>> {
    let path = repo_root().join(format!("fixtures/esp32c3-ble/{name}.bin"));
    let Ok(bytes) = std::fs::read(&path) else {
        if std::env::var("LABWIRED_REQUIRE_C3_BLE").as_deref() == Ok("1") {
            panic!(
                "{} not found but LABWIRED_REQUIRE_C3_BLE=1 — BLE connections are UNGUARDED. \
                 Run `scripts/ci/fetch-c3-ble-flash.sh fixtures/esp32c3-ble \
                 scripts/ci/{name}.sha256` first.",
                path.display()
            );
        }
        eprintln!(
            "SKIP: {} not found (fetch it; see module docs)",
            path.display()
        );
        return None;
    };
    let got = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        got,
        pinned_image_sha(&format!("{name}.sha256")),
        "{name}.bin does not match its pinned digest"
    );
    Some(bytes)
}

fn node(id: &str, firmware: Vec<u8>) -> ResolvedWorldNode {
    let root = repo_root();
    ResolvedWorldNode {
        id: id.to_string(),
        system: SystemManifest::from_file(root.join("configs/systems/esp32c3-devkit.yaml"))
            .expect("esp32c3-devkit system"),
        chip: ChipDescriptor::from_file(root.join("configs/chips/esp32c3.yaml"))
            .expect("esp32c3 chip"),
        firmware: NodeFirmware::FlashImage(firmware),
        blobs: Default::default(),
    }
}

struct Run {
    consoles: BTreeMap<String, String>,
    phone: Option<labwired_core::peripherals::ble_central::CentralReport>,
    rounds: u64,
}

/// Build the world, attach a console sink per node, and step until `done`
/// says so (checked every 200k rounds) or `max_rounds` pass.
fn run(
    env_yaml: &str,
    nodes: Vec<ResolvedWorldNode>,
    max_rounds: u64,
    done: impl Fn(&BTreeMap<String, String>, &World) -> bool,
) -> Run {
    let manifest: EnvironmentManifest = serde_yaml::from_str(env_yaml).expect("environment");
    let mut world = World::from_resolved(manifest, nodes).expect("build the world");
    let mut sinks = BTreeMap::new();
    for (id, machine) in world.machines.iter_mut() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        machine
            .attach_uart_tx_sink(sink.clone(), false)
            .expect("console sink");
        sinks.insert(id.clone(), sink);
    }
    let consoles = |sinks: &BTreeMap<String, Arc<Mutex<Vec<u8>>>>| {
        sinks
            .iter()
            .map(|(id, s)| {
                (
                    id.clone(),
                    String::from_utf8_lossy(&s.lock().unwrap()).into_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let mut rounds = 0;
    while rounds < max_rounds {
        for (id, r) in world.step_all() {
            r.unwrap_or_else(|e| panic!("node '{id}' step: {e:?}"));
        }
        rounds += 1;
        if rounds % 200_000 == 0 && done(&consoles(&sinks), &world) {
            break;
        }
    }
    Run {
        consoles: consoles(&sinks),
        phone: world
            .ble_central_reports()
            .into_iter()
            .next()
            .map(|(_, r)| r),
        rounds,
    }
}

const TWO_NODE_ENV: &str = r#"
schema_version: "1.0"
name: c3-ble-gatt
nodes:
  - { id: client, system: esp32c3-devkit.yaml, firmware: c3-ble-gatt-client-flash.bin }
  - { id: server, system: esp32c3-devkit.yaml, firmware: c3-ble-gatt-notify-flash.bin }
interconnects:
  - type: ble_air
    nodes: [client, server]
"#;

fn two_node_run() -> Option<Run> {
    let server = flash_image("c3-ble-gatt-notify-flash")?;
    let client = flash_image("c3-ble-gatt-client-flash")?;
    Some(run(
        TWO_NODE_ENV,
        vec![node("client", client), node("server", server)],
        400_000_000,
        |c, _| {
            c["client"]
                .matches("Notify callback for characteristic")
                .count()
                >= 5
        },
    ))
}

#[test]
#[ignore = "two faithful C3 ROM boots in lockstep; release + fetched fixtures"]
fn two_c3_nodes_connect_discover_read_write_and_notify() {
    let Some(a) = two_node_run() else { return };
    let client = &a.consoles["client"];
    let server = &a.consoles["server"];
    eprintln!(
        "── client ({} rounds) ──\n{client}\n── server ──\n{server}",
        a.rounds
    );

    // The client narrates every GATT procedure; each line is printed only after
    // the procedure completed over the air.
    for (step, line) in [
        ("scan", "BLE Advertised Device found: Name: ESP32"),
        ("connect", " - Connected to server"),
        ("service discovery", " - Found our service"),
        ("characteristic discovery", " - Found our characteristic"),
        ("read", "The characteristic value was: "),
        ("subscribe + connected", "We are now connected to the BLE Server."),
        ("write", "Setting new characteristic value to \"Time since boot: "),
        (
            "notify",
            "Notify callback for characteristic beb5483e-36e1-4688-b7f5-ea07361b26a8 of data length 4",
        ),
    ] {
        assert!(
            client.contains(line),
            "{step}: the client never printed {line:?}\n{client}"
        );
    }
    assert!(server.contains("Waiting a client connection to notify..."));
    assert!(
        client.matches("Notify callback for characteristic").count() >= 5,
        "fewer than five notifications reached the client"
    );

    // Deterministic: the same world twice gives the same serial, byte for byte.
    let b = two_node_run().expect("fixtures present");
    assert_eq!(
        a.rounds, b.rounds,
        "the two runs stopped at different rounds"
    );
    assert_eq!(a.consoles, b.consoles, "two identical runs diverged");
}

fn phone_env() -> String {
    format!(
        r#"
schema_version: "1.0"
name: c3-ble-phone
nodes:
  - {{ id: server, system: esp32c3-devkit.yaml, firmware: c3-ble-gatt-notify-flash.bin }}
interconnects:
  - type: ble_central
    nodes: [server]
    config:
      id: phone
      target_name: ESP32
      script:
        - connect
        - discover
        - read: {CHAR_UUID}
        - write: {{ uuid: {CHAR_UUID}, text: hello }}
        - subscribe: {CHAR_UUID}
        - wait_notify: {{ count: 3, timeout_ms: 2000 }}
        - disconnect
"#
    )
}

fn phone_run() -> Option<Run> {
    let server = flash_image("c3-ble-gatt-notify-flash")?;
    Some(run(
        &phone_env(),
        vec![node("server", server)],
        400_000_000,
        |_, w| {
            w.ble_central_reports()
                .first()
                .is_some_and(|(_, r)| r.script_done)
        },
    ))
}

#[test]
#[ignore = "a faithful C3 ROM boot; release + fetched fixture"]
fn a_scripted_phone_connects_to_the_c3_gatt_server() {
    use labwired_core::peripherals::ble_central::CentralState;
    let Some(a) = phone_run() else { return };
    let r = a
        .phone
        .clone()
        .expect("the ble_central interconnect built a central");
    for l in &r.log {
        eprintln!("[phone {:>9} us] {:<7} {}", l.t_us, l.kind, l.text);
    }
    assert!(r.script_done, "the phone's script did not finish");
    assert_eq!(r.state, CentralState::Disconnected);
    assert!(
        r.log.iter().any(|l| l.text.starts_with("TX CONNECT_IND")),
        "no CONNECT_IND"
    );
    assert!(
        r.log
            .iter()
            .any(|l| l.text == "connection established (first packet acknowledged)"),
        "the server never answered a connection event"
    );
    // Discovery found the sketch's service and characteristic, with a CCCD.
    assert!(
        r.services
            .iter()
            .any(|s| s == "4fafc201-1fb5-459e-8fcc-c5c9c331914b"),
        "service not discovered: {:?}",
        r.services
    );
    let ch = r
        .characteristics
        .iter()
        .find(|c| c.uuid == CHAR_UUID)
        .expect("characteristic discovered");
    assert_eq!(ch.properties, 0x3a, "read | write | notify | indicate");
    assert!(ch.cccd_handle.is_some(), "CCCD discovered");
    // Read: the sketch's 4-byte little-endian counter.
    assert_eq!(r.reads.len(), 1);
    assert_eq!(r.reads[0].value.len(), 4);
    // Write request acknowledged by the server's ATT.
    assert_eq!(r.writes_acked.len(), 1);
    assert_eq!(r.writes_acked[0].value, b"hello");
    // Notifications after subscribing: the counter, strictly increasing.
    assert!(
        r.notification_count >= 3,
        "{} notifications",
        r.notification_count
    );
    let counters: Vec<u32> = r
        .notifications
        .iter()
        .map(|n| u32::from_le_bytes(n.value[..4].try_into().unwrap()))
        .collect();
    assert!(
        counters.windows(2).all(|w| w[1] > w[0]),
        "notified counter not increasing: {counters:?}"
    );

    let b = phone_run().expect("fixture present");
    let b = b.phone.expect("central");
    assert_eq!(
        serde_json::to_string(&r.log).unwrap(),
        serde_json::to_string(&b.log).unwrap(),
        "two identical runs produced different phone transcripts"
    );
}
