# Released multi-node UDS runner implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a released `labwired test` runner that executes a dual-node FDCAN environment declared entirely in YAML and emits the ordinary CI artifact set.

**Architecture:** Keep the existing single-node `inputs.firmware` path untouched. Add a disjoint `inputs.env` parser branch that builds a deterministic `World`, wires named FDCAN peripherals to a `CanBus`, evaluates mandatory node-qualified memory assertions, and writes environment-aware result, snapshot, UART, and JUnit artifacts. Publish this as v0.19.0, then migrate UDSLib to the released public Action.

**Tech Stack:** Rust, serde_yaml, Cortex-M simulator, GitHub Actions release archives/GHCR, Python static CI contract tests.

---

## File structure

- `crates/config/src/lib.rs` owns strict YAML parsing and the `LoadedTestScript` union.
- `crates/core/src/peripherals/fdcan.rs`, `crates/core/src/bus/routing.rs`, and `crates/core/src/world.rs` own the post-build FDCAN attachment seam and deterministic world construction.
- `crates/cli/src/commands/test.rs` owns environment execution and artifact creation; `crates/cli/src/artifacts.rs` owns serializable environment records.
- `crates/core/tests/fdcan_multinode.rs` proves manifest wiring; `crates/cli/tests/env_runner.rs` proves release-facing output behavior.
- `Cargo.toml`, `CHANGELOG.md`, the public action metadata, and CI docs identify v0.19.0 as the released runner contract.

### Task 1: Parse a strict environment test script

**Files:**
- Modify: `crates/config/src/lib.rs:632-755,1155-1197`
- Test: `crates/config/src/lib.rs` unit-test module

- [ ] **Step 1: Write failing parser tests for environment scripts**

```rust
#[test]
fn load_env_script_selects_env_variant_and_node_assertion() {
    let script = write_temp_script(r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 7, size: 32 }
"#);
    let LoadedTestScript::Env(env) = load_test_script(&script).unwrap() else {
        panic!("expected environment script");
    };
    assert_eq!(env.inputs.env, "twonode-env.yaml");
    assert_eq!(env.assertions.len(), 1);
}

#[test]
fn env_script_requires_a_node_for_memory_assertions() {
    let script = parse_env_script_without_node();
    assert!(script.validate().unwrap_err().to_string().contains("node"));
}
```

- [ ] **Step 2: Run the focused config tests and observe that `Env` does not exist**

Run: `cargo test -p labwired-config load_env_script_selects_env_variant_and_node_assertion env_script_requires_a_node_for_memory_assertions`

Expected: compilation failure referring to the absent `LoadedTestScript::Env` variant.

- [ ] **Step 3: Add the disjoint strict schema**

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EnvTestInputs { pub env: String }

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EnvTestScript {
    pub schema_version: String,
    pub inputs: EnvTestInputs,
    pub limits: TestLimits,
    #[serde(default)] pub assertions: Vec<TestAssertion>,
}

pub enum LoadedTestScript {
    V1_0(TestScript), LegacyV1(LegacyTestScriptV1), Env(EnvTestScript),
}
```

Probe raw YAML for a string `inputs.env` before attempting `TestScript` so the
existing `#[serde(deny_unknown_fields)]` single-node contract remains intact.
Extend `MemoryValueDetails` with `#[serde(default)] pub node: Option<String>`.
`EnvTestScript::validate` must require schema 1.0, a non-empty environment
path, `max_steps > 0`, a non-empty assertion list, only `memory_value`
assertions, and `node` on every environment memory assertion.
Reject unsupported environment-only options (`no_progress_steps`, VCD/
trace-only limits, stop-on-first-pass, faults, verdicts, and stimuli) with a
configuration diagnostic; never accept an option that the world runner will
ignore.

- [ ] **Step 4: Run parser regression coverage**

Run: `cargo test -p labwired-config`

Expected: all existing config tests plus the new env tests pass.

- [ ] **Step 5: Commit the parser slice**

```bash
git add crates/config/src/lib.rs
git commit -m "feat(config): parse multi-node environment test scripts"
```

### Task 2: Wire a manifest `can_bus` to real FDCAN peripherals

**Files:**
- Modify: `crates/core/src/peripherals/fdcan.rs:271-279`
- Modify: `crates/core/src/bus/routing.rs:463-490`
- Modify: `crates/core/src/world.rs:22-215`
- Create: `crates/core/tests/fdcan_multinode.rs`
- Test: affected FDCAN/core test modules

- [ ] **Step 1: Write red tests for late binding and manifest wiring**

```rust
#[test]
fn attach_can_bus_by_id_changes_fdcan_from_unattached_to_attached() {
    let mut bus = build_test_bus_with_fdcan1();
    assert!(!fdcan(&bus, "fdcan1").is_bus_attached());
    let mut can = CanBus::new();
    let (tx, rx) = can.attach();
    bus.attach_can_bus_by_id("fdcan1", tx, rx).unwrap();
    assert!(fdcan(&bus, "fdcan1").is_bus_attached());
}

#[test]
fn world_manifest_rejects_unknown_can_bus_node() {
    let result = World::from_manifest(env_with_can_nodes(["ghost"]), fixture_root());
    assert!(result.unwrap_err().to_string().contains("ghost"));
}
```

Also add a world test that drives a TX-capable fixture node and proves a
different attached receiver gets a frame; do not accept an own-frame echo as
cross-node evidence.

- [ ] **Step 2: Run focused tests and observe the missing APIs / unsupported interconnect**

Run: `cargo test -p labwired-core attach_can_bus_by_id_changes_fdcan_from_unattached_to_attached world_manifest_rejects_unknown_can_bus_node`

Expected: compile errors for `attach_bus`/`attach_can_bus_by_id`, then an
`unsupported interconnect type 'can_bus'` failure until implementation exists.

- [ ] **Step 3: Add the narrow binding seam and stable world behavior**

```rust
impl Fdcan {
    pub fn attach_bus(&mut self, tx: Sender<CanFrame>, rx: Receiver<CanFrame>) {
        self.bus_tx = Some(tx); self.bus_rx = Some(rx);
    }
}

pub fn attach_can_bus_by_id(&mut self, id: &str, tx: Sender<CanFrame>, rx: Receiver<CanFrame>)
    -> anyhow::Result<()>;
```

Add `MachineTrait::attach_can_bus`, create one `CanBus` for each
`interconnects[type=can_bus]`, attach every named node to the configured
nonblank `config.peripheral` (there is no default), and error with the
node/peripheral name when resolution fails. Require at least two distinct
nodes, sort a copied membership list lexically before endpoint attachment, and
therefore make YAML membership permutations unable to reorder simultaneous
frame delivery. Build Cortex-M world nodes via `configure_cortex_m(&mut bus)`
instead of `CortexM::new()`. Change world machine storage/stepping and artifact
iteration to stable node-id order. The receiver test must use a node with no
transmit source so an own-frame echo cannot satisfy the cross-node proof.

- [ ] **Step 4: Run FDCAN and world tests**

Run: `cargo test -p labwired-core --test fdcan --test fdcan_multinode`

Expected: late binding, receiver delivery, unknown-node error, and existing
FDCAN tests pass.

- [ ] **Step 5: Commit the world-wiring slice**

```bash
git add crates/core/src/peripherals/fdcan.rs crates/core/src/bus/routing.rs crates/core/src/world.rs crates/core/tests/fdcan_multinode.rs
git commit -m "feat(world): wire FDCAN buses from environment manifests"
```

### Task 3: Execute environment tests and write ordinary CI artifacts

**Files:**
- Modify: `crates/cli/src/artifacts.rs:20-138`
- Modify: `crates/cli/src/commands/test.rs:35-206`
- Create: `crates/cli/tests/env_runner.rs`
- Test: `crates/cli/tests/env_runner.rs`

- [ ] **Step 1: Add failing end-to-end environment-runner tests**

Create a temp directory with a copied CI fixture ELF/chip/system, an environment
manifest, and a script using `inputs.env`. Invoke `env!("CARGO_BIN_EXE_labwired")`.

```rust
#[test]
fn env_run_pass_writes_report_artifacts() {
    let output = run_env_fixture("expected_value: 0", "out");
    assert_eq!(output.status.code(), Some(0));
    let result: Value = read_json("out/result.json");
    assert_eq!(result["status"], "pass");
    assert!(result["assertions"][0]["passed"].as_bool().unwrap());
    for name in ["uart.log", "snapshot.json", "junit.xml"] {
        assert!(out_dir().join(name).is_file(), "missing {name}");
    }
}

#[test]
fn env_run_failed_assertion_writes_fail_result_and_junit() {
    let output = run_env_fixture("expected_value: 1", "out");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(read_json("out/result.json")["status"], "fail");
    assert!(read_text("out/junit.xml").contains("<failure"));
}

#[test]
fn env_config_error_writes_machine_readable_artifacts() {
    let output = run_script_with_missing_env("out");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(read_json("out/result.json")["status"], "error");
}
```

- [ ] **Step 2: Run the new test target and observe the v0.18-style schema rejection**

Run: `cargo test -p labwired-cli --test env_runner`

Expected: the success fixture exits 2 with `unknown field 'env'` before Tasks
1–2 are integrated; after parser support it still fails until world dispatch
and artifacts are implemented.

- [ ] **Step 3: Implement a world-aware runner/output path**

Dispatch `LoadedTestScript::Env` before single-node firmware resolution. Resolve
the environment path relative to the script, build `World`, attach a capture
sink per node, step for `max_steps` rounds, and evaluate each node-qualified
memory value with the same 1/2/4-byte or 8/16/32-bit rules as the single-node
path. A node execution error is a runtime error; unsupported assertions are
configuration errors. Reject CLI `--firmware` and `--system` overrides for
environment scripts. Apply `max_cycles` to the greatest final node cycle count,
`max_uart_bytes` to the sum of capture sinks, and `wall_time_ms` to the whole
world run.

Define serializable environment-specific artifact records rather than inventing
a fake single-node CPU/firmware:

```rust
#[derive(Serialize)]
struct EnvTestConfig { environment: PathBuf, script: PathBuf }
#[derive(Serialize)]
struct EnvNodeSnapshot { id: String, cycles: u64 }
#[derive(Serialize)]
struct EnvSnapshot { r#type: String, nodes: Vec<EnvNodeSnapshot>, /* result fields */ }
```

Write `result.json`, sorted node-labelled `uart.log`, `snapshot.json`, and
JUnit both in `--output-dir` and at an explicit `--junit` path. Preserve the
outer `status`, `stop_reason`, metrics, and assertion fields consumed by the
public report renderer. Capture a deterministic aggregate firmware digest from
sorted node id/path/content tuples. Make the JSON `config` name the environment
path (not a fictitious single firmware), and make the snapshot explicitly
`type: "environment"` with sorted per-node final-state records.

- [ ] **Step 4: Run CLI artifact and renderer compatibility tests**

Run:

```bash
cargo test -p labwired-cli --test env_runner --test outputs --test script_validation
python3 .github/actions/labwired-test/test_render_report.py
```

Expected: pass/fail/config-error environment runs all emit renderable artifacts;
existing single-node output behavior and report rendering remain green.

- [ ] **Step 5: Commit the runner/artifact slice**

```bash
git add crates/cli/src/artifacts.rs crates/cli/src/commands/test.rs crates/cli/tests/env_runner.rs
git commit -m "feat(cli): run multi-node environment tests with artifacts"
```

### Task 4: Validate the real UDS gate, release v0.19.0, and migrate CI

**Files:**
- Modify: `Cargo.toml:97`, `CHANGELOG.md`, `.github/actions/labwired-test/action.yml`, public Core CI docs/examples
- Modify later in UDSLib: `.github/workflows/nightly-h5-gate.yml`, H5 descriptor files, tester timing expectation, README, and CI contract test

- [ ] **Step 1: Run the actual consumer acceptance before release**

Build the UDSLib tester and ECU ELFs, then run the locally built Core CLI:

```bash
export UDSLIB_DIR=/Users/andrii/.config/superpowers/worktrees/udslib/uds-ci-report-proof
export RUST_LLD="$(find "$(rustc --print sysroot)" -name rust-lld -type f -print -quit)"
make -B -C "$UDSLIB_DIR/examples/h5_uds_ecu_full/firmware"
make -B -C "$UDSLIB_DIR/examples/h5_uds_tester/firmware"
cargo build -p labwired-cli --release
target/release/labwired test --script "$UDSLIB_DIR/examples/h5_uds_tester/allservices-gate.yaml" --output-dir /tmp/labwired-uds-preflight --no-uart-stdout
jq -e '.status == "pass"' /tmp/labwired-uds-preflight/result.json
rg -F 'SERVICES 27/27 PASS' /tmp/labwired-uds-preflight/uart.log
```

Expected: exit 0, passing memory oracle, and authentic 27/27 transcript. If
this exposes a decoder defect, add a narrow failing Core decoder test before
fixing it; do not copy historical commits blindly.

- [ ] **Step 2: Bump and document the immutable release**

Set workspace version and public Action default to `v0.19.0`; add a concise
CHANGELOG entry for released multi-node YAML/FDCAN CI support and update public
examples from v0.18.0 to v0.19.0. Keep Action source examples full-commit-SHA
pinned.

- [ ] **Step 3: Run release-candidate verification**

Run:

```bash
cargo fmt --check
cargo test -p labwired-config
cargo test -p labwired-core --test fdcan --test fdcan_multinode
cargo test -p labwired-cli --test env_runner --test outputs --test script_validation
scripts/ci/verify-release-runner-contract.sh
python3 .github/actions/labwired-test/test_render_report.py
```

Expected: all commands exit 0.

- [ ] **Step 4: Commit and merge the release preparation**

```bash
git add Cargo.toml CHANGELOG.md .github/actions/labwired-test/action.yml docs
git commit -m "release: prepare v0.19.0 multi-node CI runner"
```

After required GitHub checks pass, merge the Core PR, tag the merged main commit
as `v0.19.0`, and wait for release archives, GHCR image, and release smoke.

- [ ] **Step 5: Migrate UDSLib only after release smoke**

Update the UDSLib workflow to the immutable Action source SHA and
`version: v0.19.0`; keep only firmware compilation and the Action step.
Vendor the released descriptor in both H5 examples, correct the P2* expected
bytes to `00 C8`, correct stale README guidance, and make its static contract
download/run the exact release on a small preflight where feasible. Merge it,
dispatch the nightly gate, and require `SERVICES 27/27 PASS` before publishing
the landing proof.
