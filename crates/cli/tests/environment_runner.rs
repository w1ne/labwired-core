// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! Black-box contract tests for the released multi-node `labwired test` runner.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn unique_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before Unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "labwired-environment-runner-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temporary environment directory");
    dir
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates directory")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn write_two_node_environment(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    // Deliberately list beta first: the runner contract serializes and captures
    // nodes in lexical id order, not input-manifest order.
    write_two_node_environment_in_order(dir, &["beta", "alpha"])
}

fn write_two_node_environment_in_order(
    dir: &Path,
    node_order: &[&str],
) -> (PathBuf, PathBuf, PathBuf) {
    assert_eq!(node_order.len(), 2, "fixture world must contain two nodes");
    let root = workspace_root();
    let firmware = std::fs::canonicalize(root.join("tests/fixtures/uart-ok-thumbv7m.elf"))
        .expect("fixture firmware");
    let system = std::fs::canonicalize(root.join("configs/systems/ci-fixture-uart1.yaml"))
        .expect("fixture system manifest");
    let environment = dir.join("two-node.yaml");

    let nodes = node_order
        .iter()
        .map(|id| {
            format!(
                "  - id: {id}\n    system: \"{}\"\n    firmware: \"{}\"\n",
                system.display(),
                firmware.display(),
            )
        })
        .collect::<String>();
    std::fs::write(
        &environment,
        format!(
            r#"schema_version: "1.0"
name: fixture-world
nodes:
{}"#,
            nodes,
        ),
    )
    .expect("write environment manifest");

    (environment, firmware, system)
}

/// A valid mixed world: `alpha` writes unmapped UART MMIO, while `beta` runs a
/// supported UART system and deliberately halts. This keeps fidelity evidence
/// and a runtime error independent, rather than relying on an invalid image
/// memory map to manufacture both outcomes.
fn write_fidelity_and_halt_two_node_environment(dir: &Path) -> PathBuf {
    let root = workspace_root();
    let unmapped_uart_firmware =
        std::fs::canonicalize(root.join("tests/fixtures/uart-then-bkpt-thumbv7m.elf"))
            .expect("unmapped UART fixture firmware");
    let halting_firmware =
        std::fs::canonicalize(root.join("tests/fixtures/uart-then-bkpt-thumbv7m.elf"))
            .expect("halting fixture firmware");
    let supported_system =
        std::fs::canonicalize(root.join("configs/systems/ci-fixture-uart1.yaml"))
            .expect("supported UART system fixture");
    let chip = dir.join("tiny-chip.yaml");
    let system = dir.join("tiny-system.yaml");
    let environment = dir.join("tiny-two-node.yaml");

    std::fs::write(
        &chip,
        r#"name: "tiny"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x0
  size: "128KB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals: []
"#,
    )
    .expect("write tiny chip");
    std::fs::write(
        &system,
        r#"name: "tiny-system"
chip: "tiny-chip.yaml"
"#,
    )
    .expect("write tiny system");
    std::fs::write(
        &environment,
        format!(
            r#"schema_version: "1.0"
name: tiny-world
nodes:
  - id: alpha
    system: "tiny-system.yaml"
    firmware: "{}"
  - id: beta
    system: "{}"
    firmware: "{}"
"#,
            unmapped_uart_firmware.display(),
            supported_system.display(),
            halting_firmware.display(),
        ),
    )
    .expect("write tiny environment manifest");

    environment
}

fn assert_sha256(value: &serde_json::Value) {
    let value = value.as_str().expect("SHA-256 string");
    assert_eq!(value.len(), 64, "SHA-256 must be 64 hex characters");
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "SHA-256 must be lowercase hexadecimal: {value}"
    );
}

fn run_environment_script(dir: &Path, script: &str, extra_args: &[&str]) -> std::process::Output {
    run_environment_script_with_env(dir, script, extra_args, &[])
}

fn run_environment_script_with_env(
    dir: &Path,
    script: &str,
    extra_args: &[&str],
    environment: &[(&str, &str)],
) -> std::process::Output {
    let script_path = dir.join("gate.yaml");
    std::fs::write(&script_path, script).expect("write environment test script");
    let output_dir = dir.join("artifacts");

    let mut command = Command::new(env!("CARGO_BIN_EXE_labwired"));
    command
        .arg("test")
        .arg("--script")
        .arg(&script_path)
        .arg("--no-uart-stdout")
        .arg("--output-dir")
        .arg(&output_dir);
    command.args(extra_args);
    command.envs(environment.iter().copied());
    command.output().expect("run labwired environment test")
}

#[derive(Debug)]
struct CapturedApiRequest {
    path: String,
    body: serde_json::Value,
}

struct MeteringApiServer {
    base: String,
    shutdown_tx: mpsc::Sender<()>,
    request_started_rx: mpsc::Receiver<()>,
    handle: Option<thread::JoinHandle<Vec<CapturedApiRequest>>>,
}

impl MeteringApiServer {
    fn base_url(&self) -> &str {
        &self.base
    }

    fn wait_until_request_started(&self) {
        self.request_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("wait for local metering API request to start");
    }

    fn shutdown_and_join(&mut self) -> Option<thread::Result<Vec<CapturedApiRequest>>> {
        let _ = self.shutdown_tx.send(());
        self.handle.take().map(|handle| handle.join())
    }

    fn stop_and_join(mut self) -> Vec<CapturedApiRequest> {
        self.shutdown_and_join()
            .expect("local metering API thread handle")
            .expect("local metering API thread")
    }
}

impl Drop for MeteringApiServer {
    fn drop(&mut self) {
        // On an assertion panic, wait for the helper to release its listener
        // and any bounded in-flight read before unwinding the test.
        let _ = self.shutdown_and_join();
    }
}

/// Starts a tiny, deliberately strict API double. It must see key validation
/// before the run record, validates only the known test key, and stays alive
/// until the caller signals that its client process has completed.
fn start_metering_api_server() -> MeteringApiServer {
    const TEST_KEY: &str = "environment-metering-test-key";

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local metering API");
    listener
        .set_nonblocking(true)
        .expect("make local metering API nonblocking");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("local API address")
    );
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::sync_channel(0);
    let (request_started_tx, request_started_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        ready_tx
            .send(())
            .expect("signal local metering API thread started");
        let mut requests = Vec::new();

        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // Accepted sockets inherit the listener's nonblocking
                    // mode on this platform. The listener needs polling, but
                    // a complete HTTP request needs blocking reads bounded by
                    // `read_api_request`'s timeout.
                    stream
                        .set_nonblocking(false)
                        .expect("make accepted local API stream blocking");
                    request_started_tx
                        .send(())
                        .expect("signal local metering API request start");
                    let request = read_api_request(&mut stream);
                    let is_valid_key =
                        request.path == "/v1/keys/validate" && request.body["api_key"] == TEST_KEY;
                    let response = if request.path == "/v1/keys/validate" && is_valid_key {
                        r#"{"valid":true,"workspace_id":"test-workspace","plan":"pro","cycles_quota":1000,"cycles_used_mtd":0}"#
                    } else if request.path == "/v1/keys/validate" {
                        r#"{"valid":false}"#
                    } else {
                        r#"{}"#
                    };
                    write_api_response(&mut stream, response);
                    requests.push(request);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    match shutdown_rx.recv_timeout(Duration::from_millis(10)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
                Err(error) => panic!("accept local metering API request: {error}"),
            }
        }

        requests
    });

    ready_rx
        .recv()
        .expect("wait for local metering API thread to start");

    MeteringApiServer {
        base,
        shutdown_tx,
        request_started_rx,
        handle: Some(handle),
    }
}

fn read_api_request(stream: &mut TcpStream) -> CapturedApiRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set local API read timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("clone local API stream"));
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .expect("read local API request line");
    let path = request_line
        .split_whitespace()
        .nth(1)
        .expect("local API request path")
        .to_string();

    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .expect("read local API request header");
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().expect("parse request content length");
            }
        }
    }

    let mut body = vec![0_u8; content_length];
    reader
        .read_exact(&mut body)
        .expect("read local API request body");
    CapturedApiRequest {
        path,
        body: serde_json::from_slice(&body).expect("parse local API request JSON"),
    }
}

fn write_api_response(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .expect("write local API response");
    stream.flush().expect("flush local API response");
}

#[test]
fn metering_api_server_waits_for_the_client_instead_of_an_elapsed_deadline() {
    const TEST_KEY: &str = "environment-metering-test-key";

    let server = start_metering_api_server();
    let api_base = server.base_url().to_owned();
    // A busy test machine can schedule the child after the old two-second
    // listener deadline. The server must remain available until this test has
    // completed its client work, rather than treating elapsed time as a
    // completion signal.
    thread::sleep(Duration::from_millis(2_500));

    let address = api_base
        .strip_prefix("http://")
        .expect("local metering API base URL");
    let body = format!(r#"{{"api_key":"{TEST_KEY}"}}"#);
    let request = format!(
        "POST /v1/keys/validate HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut client =
        TcpStream::connect(address).expect("metering API must still accept a delayed test client");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set delayed client response timeout");
    client
        .write_all(request.as_bytes())
        .expect("send delayed metering API request");
    let mut response = String::new();
    client
        .read_to_string(&mut response)
        .expect("read delayed metering API response");
    assert!(response.starts_with("HTTP/1.1 200 OK"));

    let requests = server.stop_and_join();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/keys/validate");
    assert_eq!(requests[0].body["api_key"], TEST_KEY);
}

#[test]
fn metering_api_server_drop_waits_for_an_inflight_request() {
    const TEST_KEY: &str = "environment-metering-test-key";

    let server = start_metering_api_server();
    let address = server
        .base_url()
        .strip_prefix("http://")
        .expect("local metering API base URL")
        .to_owned();
    let body = format!(r#"{{"api_key":"{TEST_KEY}"}}"#);
    let initial_request = format!(
        "POST /v1/keys/validate HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    let mut client = TcpStream::connect(&address).expect("connect delayed local API client");
    client
        .write_all(initial_request.as_bytes())
        .expect("send incomplete local API request");
    server.wait_until_request_started();

    let (finisher_ready_tx, finisher_ready_rx) = mpsc::sync_channel(0);
    let finisher = thread::spawn(move || {
        finisher_ready_tx
            .send(())
            .expect("signal delayed client readiness");
        thread::sleep(Duration::from_millis(250));
        client
            .write_all(format!("\r\n{body}").as_bytes())
            .expect("finish delayed local API request");
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set delayed client response timeout");
        let mut response = String::new();
        client
            .read_to_string(&mut response)
            .expect("read delayed local API response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
    });
    finisher_ready_rx
        .recv()
        .expect("wait for delayed client readiness");

    let started = Instant::now();
    drop(server);
    let elapsed = started.elapsed();
    finisher.join().expect("delayed client thread");
    assert!(
        elapsed >= Duration::from_millis(150),
        "dropping the helper must join the in-flight request thread; elapsed={elapsed:?}"
    );
}

#[test]
fn environment_runner_writes_sorted_real_world_artifacts() {
    let dir = unique_dir("artifacts");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 100
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
      size: 8
      mask: 0xff
  - memory_value:
      node: beta
      address: 0x20000000
      expected_value: 0
      size: 16
      mask: 0xffff
"#,
        &[],
    );

    assert!(
        output.status.success(),
        "environment run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["result_schema_version"], "1.0-environment");
    assert_eq!(result["run_type"], "environment");
    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "max_steps");
    assert_eq!(result["steps_executed"], 100);
    assert_eq!(result["instructions"], 200);
    assert!(result["config"].get("firmware").is_none());
    assert!(result["config"]["environment"]
        .as_str()
        .expect("environment provenance")
        .ends_with("two-node.yaml"));
    let nodes = result["config"]["nodes"]
        .as_array()
        .expect("per-node provenance");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["id"], "alpha");
    assert_eq!(nodes[1]["id"], "beta");
    assert!(nodes
        .iter()
        .all(|node| node["firmware_hash"].as_str().is_some()));
    assert!(nodes
        .iter()
        .all(|node| node["system_hash"].as_str().is_some()));
    assert_sha256(&result["config"]["world_firmware_hash"]);

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");
    let snapshot_nodes = snapshot["nodes"].as_array().expect("environment nodes");
    assert_eq!(snapshot_nodes[0]["id"], "alpha");
    assert_eq!(snapshot_nodes[1]["id"], "beta");
    assert!(snapshot_nodes[0]["state"]["cpu"].is_object());
    assert_eq!(snapshot_nodes[0]["cycles"], result["cycles"]);
    assert_eq!(snapshot_nodes[1]["cycles"], result["cycles"]);
    assert!(snapshot_nodes
        .iter()
        .all(|node| node["cycles"].as_u64().is_some_and(|cycles| cycles > 0)));

    let uart = std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log");
    assert!(uart.starts_with("[node:alpha]\n"));
    let beta = uart.find("[node:beta]\n").expect("beta UART section");
    assert!(
        beta > 0,
        "UART sections must be sorted by node id: {uart:?}"
    );
    assert_eq!(uart.matches("[node:").count(), 2);
    assert!(uart.contains("OK\n"));

    let junit = std::fs::read_to_string(output_dir.join("junit.xml")).expect("read junit.xml");
    assert!(junit.contains("name=\"run\""));
    assert!(junit.contains("name=\"assertion 1:"));
    assert!(junit.contains("name=\"assertion 2:"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_records_aggregate_world_metering_after_key_validation() {
    const TEST_KEY: &str = "environment-metering-test-key";

    let dir = unique_dir("api-metering");
    write_two_node_environment(&dir);
    let server = start_metering_api_server();
    let api_base = server.base_url().to_owned();
    let output = run_environment_script_with_env(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 10
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
        &[
            ("LABWIRED_API_KEY", TEST_KEY),
            ("LABWIRED_API_BASE", &api_base),
        ],
    );
    assert!(
        output.status.success(),
        "environment run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    let requests = server.stop_and_join();
    assert_eq!(
        requests.len(),
        2,
        "expected key validation followed by a run meter record, got {requests:?}"
    );
    assert_eq!(requests[0].path, "/v1/keys/validate");
    assert_eq!(requests[0].body["api_key"], TEST_KEY);
    assert_eq!(requests[1].path, "/v1/runs");
    assert_eq!(
        requests[1].body["firmware_hash"], result["config"]["world_firmware_hash"],
        "metering must use the aggregate world firmware hash"
    );
    assert_eq!(requests[1].body["cycles"], result["cycles"]);
    assert_eq!(requests[1].body["exit_status"], 0);
    assert_eq!(requests[1].body["api_key"], TEST_KEY);
    assert!(requests[1].body["duration_ms"].as_u64().is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_unknown_assertion_node_is_a_config_error_with_environment_artifacts() {
    let dir = unique_dir("unknown-node");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
assertions:
  - memory_value:
      node: missing
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "error");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["message"]
        .as_str()
        .expect("config error message")
        .contains("nonexistent node 'missing'"));
    assert!(result["config"].get("firmware").is_none());
    assert_eq!(result["config"]["nodes"][0]["id"], "alpha");
    assert_eq!(result["config"]["nodes"][1]["id"], "beta");

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");
    assert_eq!(snapshot["status"], "error");
    assert_eq!(snapshot["nodes"][0]["id"], "alpha");
    assert_eq!(snapshot["nodes"][1]["id"], "beta");
    assert_eq!(snapshot["nodes"][0]["cycles"], 0);
    assert_eq!(snapshot["nodes"][1]["cycles"], 0);

    let uart = std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log");
    assert_eq!(uart, "[node:alpha]\n[node:beta]\n");
    let junit = std::fs::read_to_string(output_dir.join("junit.xml")).expect("read junit.xml");
    assert!(junit.contains("name=\"run\""));
    assert!(junit.contains("config error"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_parser_config_error_keeps_environment_provenance() {
    let dir = unique_dir("parser-config-error");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
  no_progress_steps: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["config"].get("firmware").is_none());
    assert!(result["config"]["environment"]
        .as_str()
        .expect("environment provenance")
        .ends_with("two-node.yaml"));
    assert_eq!(result["config"]["nodes"][0]["id"], "alpha");
    assert_eq!(result["config"]["nodes"][1]["id"], "beta");

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");
    assert_eq!(
        std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log"),
        "[node:alpha]\n[node:beta]\n"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_unusable_explicit_env_values_keep_environment_artifacts() {
    let mut empty_world_hashes = Vec::new();

    for (label, env_value) in [("null", "null"), ("number", "42")] {
        let dir = unique_dir(&format!("invalid-env-{label}"));
        let output = run_environment_script(
            &dir,
            &format!(
                r#"schema_version: "1.0"
inputs:
  env: {env_value}
limits:
  max_steps: 1
assertions: []
"#
            ),
            &[],
        );

        assert_eq!(
            output.status.code(),
            Some(2),
            "{label} env value: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output_dir = dir.join("artifacts");
        let result: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
        )
        .expect("parse result.json");
        assert_eq!(result["stop_reason"], "config_error");
        assert!(result["config"].get("firmware").is_none());
        assert!(result["config"]["environment"]
            .as_str()
            .expect("placeholder environment provenance")
            .ends_with("__labwired_invalid_inputs_env__.yaml"));
        assert!(result["config"]["nodes"]
            .as_array()
            .expect("environment nodes")
            .is_empty());
        assert_sha256(&result["config"]["world_firmware_hash"]);
        empty_world_hashes.push(
            result["config"]["world_firmware_hash"]
                .as_str()
                .expect("world firmware hash")
                .to_owned(),
        );

        let snapshot: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
        )
        .expect("parse snapshot.json");
        assert_eq!(snapshot["type"], "environment");
        assert_eq!(snapshot["status"], "error");
        assert!(snapshot["nodes"]
            .as_array()
            .expect("snapshot nodes")
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log"),
            ""
        );
        let junit = std::fs::read_to_string(output_dir.join("junit.xml")).expect("read junit.xml");
        assert!(junit.contains("config error"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    assert_eq!(
        empty_world_hashes[0], empty_world_hashes[1],
        "an empty environment world has deterministic provenance"
    );
}

#[test]
fn environment_runner_world_firmware_hash_is_order_independent() {
    let first_dir = unique_dir("world-firmware-hash-first");
    let second_dir = unique_dir("world-firmware-hash-second");
    write_two_node_environment_in_order(&first_dir, &["beta", "alpha"]);
    write_two_node_environment_in_order(&second_dir, &["alpha", "beta"]);
    let script = r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#;

    let first_output = run_environment_script(&first_dir, script, &[]);
    let second_output = run_environment_script(&second_dir, script, &[]);
    assert!(
        first_output.status.success(),
        "first world run failed: {}",
        String::from_utf8_lossy(&first_output.stderr)
    );
    assert!(
        second_output.status.success(),
        "second world run failed: {}",
        String::from_utf8_lossy(&second_output.stderr)
    );

    let first: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(first_dir.join("artifacts/result.json"))
            .expect("read first result.json"),
    )
    .expect("parse first result.json");
    let second: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(second_dir.join("artifacts/result.json"))
            .expect("read second result.json"),
    )
    .expect("parse second result.json");
    assert_sha256(&first["config"]["world_firmware_hash"]);
    assert_sha256(&second["config"]["world_firmware_hash"]);
    assert_eq!(
        first["config"]["world_firmware_hash"], second["config"]["world_firmware_hash"],
        "manifest declaration order must not change the world firmware identity"
    );

    let _ = std::fs::remove_dir_all(&first_dir);
    let _ = std::fs::remove_dir_all(&second_dir);
}

#[test]
fn environment_runner_rejects_single_machine_firmware_and_system_overrides() {
    let dir = unique_dir("overrides");
    let (_environment, firmware, system) = write_two_node_environment(&dir);
    let firmware_text = firmware.display().to_string();
    let system_text = system.display().to_string();
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &["--firmware", &firmware_text, "--system", &system_text],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["stop_reason"], "config_error");
    let message = result["message"].as_str().expect("config error message");
    assert!(message.contains("--firmware"));
    assert!(message.contains("--system"));
    assert!(message.contains("topology comes exclusively from inputs.env"));
    assert!(result["config"].get("firmware").is_none());
    assert_eq!(result["config"]["nodes"].as_array().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_memory_assertions_keep_single_node_u32_mask_semantics() {
    let dir = unique_dir("memory-mask-semantics");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0x100
      size: 8
"#,
        &[],
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "an 8-bit zero must not equal an unmasked u32 expected value: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "fail");
    assert_eq!(result["assertions"][0]["passed"], false);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_rejects_explicit_default_trace_max() {
    let dir = unique_dir("explicit-trace-max");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &["--trace-max", "100000"],
    );

    assert_eq!(output.status.code(), Some(2));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["message"]
        .as_str()
        .expect("config error message")
        .contains("--trace/--vcd/--trace-max"));
    assert!(result["config"].get("firmware").is_none());
    assert_eq!(result["config"]["nodes"].as_array().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_environment_script_with_recognizable_env_keeps_environment_artifacts() {
    let dir = unique_dir("malformed-environment-script");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: [1
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["config"].get("firmware").is_none());
    assert!(result["config"]["environment"]
        .as_str()
        .expect("environment provenance")
        .ends_with("two-node.yaml"));
    assert_eq!(result["config"]["nodes"][0]["id"], "alpha");
    assert_eq!(result["config"]["nodes"][1]["id"], "beta");

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");
    assert_eq!(
        std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log"),
        "[node:alpha]\n[node:beta]\n"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_inline_environment_script_keeps_environment_artifacts() {
    let dir = unique_dir("malformed-inline-environment-script");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs: { env: "two-node.yaml" }
limits:
  max_steps: [1
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["run_type"], "environment");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["config"].get("firmware").is_none());
    assert!(result["config"]["environment"]
        .as_str()
        .expect("environment provenance")
        .ends_with("two-node.yaml"));

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_script_without_a_direct_inputs_env_keeps_legacy_artifacts() {
    let dir = unique_dir("ambiguous-malformed-script");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"notes: |
  inputs:
    env: "two-node.yaml"
limits:
  max_steps: [1
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert!(
        result["config"].get("firmware").is_some(),
        "a scalar mentioning inputs.env must not be reclassified as an environment run"
    );

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "config_error");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_reports_world_cycle_and_total_uart_limits_truthfully() {
    let cycle_dir = unique_dir("max-cycles");
    let (_environment, _firmware, _system) = write_two_node_environment(&cycle_dir);
    let cycle_output = run_environment_script(
        &cycle_dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 100
  max_cycles: 2
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );
    assert!(
        cycle_output.status.success(),
        "cycle-limited world failed: {}",
        String::from_utf8_lossy(&cycle_output.stderr)
    );
    let cycle_result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(cycle_dir.join("artifacts/result.json"))
            .expect("read cycle result"),
    )
    .expect("parse cycle result");
    assert_eq!(cycle_result["stop_reason"], "max_cycles");
    assert_eq!(
        cycle_result["stop_reason_details"]["triggered_limit"]["name"],
        "max_cycles"
    );
    assert_eq!(
        cycle_result["stop_reason_details"]["triggered_limit"]["value"],
        2
    );
    assert!(cycle_result["cycles"].as_u64().unwrap() >= 2);
    assert_eq!(cycle_result["instructions"], 4);

    let uart_dir = unique_dir("max-uart");
    let (_environment, _firmware, _system) = write_two_node_environment(&uart_dir);
    let uart_output = run_environment_script(
        &uart_dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1000
  max_uart_bytes: 1
assertions:
  - memory_value:
      node: beta
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );
    assert_eq!(
        uart_output.status.code(),
        Some(1),
        "a max_uart_bytes safety stop must fail even when memory assertions pass: {}",
        String::from_utf8_lossy(&uart_output.stderr)
    );
    let uart_result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(uart_dir.join("artifacts/result.json")).expect("read UART result"),
    )
    .expect("parse UART result");
    assert_eq!(uart_result["status"], "fail");
    assert_eq!(uart_result["stop_reason"], "max_uart_bytes");
    assert_eq!(
        uart_result["stop_reason_details"]["triggered_limit"]["name"],
        "max_uart_bytes"
    );
    assert_eq!(
        uart_result["stop_reason_details"]["triggered_limit"]["value"],
        1
    );
    assert!(
        uart_result["stop_reason_details"]["observed"]["value"]
            .as_u64()
            .unwrap()
            >= 1
    );
    assert!(uart_result["instructions"].as_u64().unwrap() >= 2);
    let junit =
        std::fs::read_to_string(uart_dir.join("artifacts/junit.xml")).expect("read UART junit");
    assert!(junit.contains("failures=\"1\""));
    assert!(junit.contains("errors=\"0\""));

    let _ = std::fs::remove_dir_all(&cycle_dir);
    let _ = std::fs::remove_dir_all(&uart_dir);
}

#[test]
fn environment_runner_enforces_uart_safety_limit_on_the_final_world_round() {
    let dir = unique_dir("max-uart-final-round");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 21
  max_uart_bytes: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "the UART safety limit must fail even when it is crossed on the final allowed world round: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "max_uart_bytes");
    assert_eq!(result["steps_executed"], 21);
    assert!(
        result["stop_reason_details"]["observed"]["value"]
            .as_u64()
            .is_some_and(|bytes| bytes >= 1),
        "the result must report the bytes that crossed the safety limit: {result}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_enforces_cycle_limit_on_the_final_world_round() {
    let dir = unique_dir("max-cycles-final-round");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
  max_cycles: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert!(
        output.status.success(),
        "a cycle-limited world must preserve its normal passing status: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "max_cycles");
    assert_eq!(result["steps_executed"], 1);
    assert_eq!(result["cycles"], 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_wall_time_safety_stop_fails_even_when_assertions_pass() {
    let dir = unique_dir("wall-time-safety-stop");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 100
  wall_time_ms: 0
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "wall_time");
    assert_eq!(result["assertions"][0]["passed"], true);
    let junit = std::fs::read_to_string(dir.join("artifacts/junit.xml")).expect("read junit");
    assert!(junit.contains("failures=\"1\""));
    assert!(junit.contains("errors=\"0\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_prioritizes_failed_assertions_over_runtime_errors() {
    let dir = unique_dir("assertion-before-runtime-error");
    write_fidelity_and_halt_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "tiny-two-node.yaml"
limits:
  max_steps: 100
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 1
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "halt");
    assert_eq!(result["assertions"][0]["passed"], false);
    let junit = std::fs::read_to_string(dir.join("artifacts/junit.xml")).expect("read junit");
    assert!(junit.contains("failures=\"1\""));
    assert!(junit.contains("errors=\"0\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_surfaces_fidelity_gaps_from_world_execution() {
    let dir = unique_dir("fidelity");
    write_fidelity_and_halt_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "tiny-two-node.yaml"
limits:
  max_steps: 100
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(3));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    let fidelity = result["fidelity"]
        .as_array()
        .expect("environment result must surface fidelity gaps");
    let mmio = fidelity
        .iter()
        .find(|gap| gap["kind"] == "unmapped_mmio")
        .expect("tiny world must report its unmapped UART MMIO access");
    assert!(mmio["address"]
        .as_str()
        .is_some_and(|address| address.starts_with("0x")));
    assert_eq!(mmio["detail"], "write");

    let _ = std::fs::remove_dir_all(&dir);
}
