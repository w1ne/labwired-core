# `run` CLI Stimulus Design

**Date:** 2026-09-16

## Purpose

Declarative input stimuli (test-script schema 1.2) exist only behind
`labwired test --script`: the `stimuli:` YAML block. Agents and scripts that
invoke the CLI directly must author or synthesize a YAML file to drive a sensor
mid-run. This design adds a first-class CLI surface on `labwired run` that
accepts stimuli as command-line arguments with the same wire fields the MCP
`run` tool exposes.

The surface is not a straight flag addition because `labwired run` today builds
its bus without a system manifest: the ARM and RISC-V paths synthesize
`external_devices: []` (crates/cli/src/commands/run.rs:1117-1120), and the
Xtensa paths never read one. External SimInput devices — the devices stimuli
target — are therefore absent from a `run` bus. So `run` gains an optional
`--system` manifest handled by a system-aware driver, and the stimulus flag
lives on that driver.

This revision incorporates review findings: arch support is scoped explicitly,
pre-run validation gets a real read-only resolution API, the driver installs no
hidden assumptions about cycle counters, and the promised output surface
matches what the code can actually deliver.

## Goals

1. `labwired run --system <manifest>` selects a system-aware driver that
   attaches external devices; the chip descriptor is resolved from the
   manifest, so `--chip` is optional when `--system` is present.
2. `--stimulus <JSON>` is repeatable and spells exactly the agent-facing MCP
   schema: `{"channel": "...", "value": 2.0, "after_cycles": 3000000, "component": "..."}`.
   `after_cycles` omitted or `0` means `at_start` (verified against
   packages/mcp/src/handlers/run.ts:44-63 and packages/api/src/agent.ts:227;
   camelCase `afterCycles` exists only in internal layers, never on the agent
   wire).
3. Stimuli are validated against the live bus before stepping. Malformed JSON,
   unknown channels, ambiguous channels, and out-of-range values fail with
   `EXIT_CONFIG_ERROR`; under `--json` the error payload includes the available
   `{component, channel}` inventory so an agent can self-correct.
4. `run` and `test` share one stimulus runtime helper, so trigger semantics
   (`at_start`, `after_cycles`, once-only firing, batch deadline limiting)
   cannot drift between them.
5. `labwired run` behavior is unchanged when `--system` is absent. The only
   mechanical change to existing paths is `--chip` becoming conditionally
   required; regression tests cover the no-`--system` invocations.

## Non-goals

- Live or interactive driving (REPL, pause/step/set-input). CLI flags inject
  timed stimuli; they do not make `run` a server.
- No semantic or CLI change to `labwired test` or the test-script schema. The
  internal `execute_test_loop` refactor in goal 4 is behavior-preserving.
- No change to the MCP temp-script synthesis flow.
- `--rom-boot`, `--break-at`, and `--watch-mem` remain exclusive to the
  existing chip paths. So does `--gpio-trace`: its observer is
  ESP32-S3-specific today (crates/cli/src/gpio_observer.rs:12,
  crates/cli/src/commands/run.rs:566-583), so passing it with `--system` is an
  error, not a silent no-op.
- `--stimulus` support without `--system` (hard error, see below).
- New SimInput devices, channels, or trigger kinds. `on_write` / `on_read`
  remain rejected for stimuli, matching test-script validation.
- RISC-V and Xtensa `--system` support in this phase (see Arch Support).

## Arch Support

The system-aware driver reuses the ARM machine construction that currently
backs `run_interactive_arm` (`configure_cortex_m` + `Machine::load_firmware`,
crates/cli/src/commands/run.rs:1182-1219). It does **not** reuse the other
`run_interactive_*` bodies, because they are not boot-complete for
sensor-bearing systems:

| Manifest arch | Phase 1 | Evidence |
| --- | --- | --- |
| `arm` | **Supported** | `run_interactive_arm` is `Machine` + `load_firmware` (run.rs:1188-1201); KW41Z and STM32 labs are ARM |
| `riscv` (ESP32-C3) | **Rejected** | `test` needs C3 behavioral stubs (commands/test.rs:819-841) and `.data` unpack + SP seeding (test.rs:950-1156) that `run_interactive_riscv` does not provide |
| `xtensa-lx7` (ESP32-S3) | **Rejected** | `test`/`run` use `fast_boot` + IDF handshake pre-paint (run.rs:638-679), not plain `load_firmware` |
| `xtensa-lx6` (classic ESP32) | **Rejected** | `configure_xtensa` is an S3 shim (crates/core/src/system/xtensa/mod.rs:23-26); the LX6 path exists only in `run_firmware_esp32`, which attaches no manifest |

An unsupported arch exits `EXIT_CONFIG_ERROR` with the arch name and the
supported set. Lifting the `test.rs` boot preparation into a shared module so
RISC-V/Xtensa can join is an explicit follow-up, not part of this plan.

## CLI Surface

```console
$ labwired run \
    --system configs/systems/frdm-kw41z-lcd.yaml \
    --firmware tests/fixtures/kw41z-lcd-activity.elf \
    --max-steps 6000000 \
    --stimulus '{"component":"fxos8700","channel":"x","value":2.0,"after_cycles":3000000}'
```

New `RunArgs` fields:

- `--system <PATH>`: board manifest (SystemManifest YAML). Selects the
  system-aware driver.
- `--stimulus <JSON>`: repeatable, order preserved. Each argument deserializes
  into a struct with `deny_unknown_fields`; missing required fields, unknown
  fields, non-integer/negative `after_cycles`, and out-of-range values are
  configuration errors reported with the argument index
  (e.g. `--stimulus[2]: ...`).

Rules:

- `--chip` is `required_unless_present("system")` and `conflicts_with("system")`.
  With `--system`, the chip YAML is resolved from the manifest.
- `--system` requires an explicit `--max-steps` budget. The driver refuses to
  run unbounded: a timed stimulus aimed at firmware that never halts would
  otherwise stream UART forever with no structured outcome. This is a
  deliberate divergence from the chip paths' unlimited default, justified for
  an agent-facing surface.
- `--stimulus` without `--system` exits `EXIT_CONFIG_ERROR` with the message
  that stimuli target devices declared in a system manifest.
- `--gpio-trace` with `--system` exits `EXIT_CONFIG_ERROR` ("only supported on
  the ESP32-S3 run path").
- A bare `channel` exposed by more than one device is an error until
  `component` disambiguates it, mirroring the bus resolution rule. Note that
  the simulator treats `component` as an exact filter: a non-matching hint is a
  `NoDevice` error, stricter than MCP's "advisory hint" wording
  (packages/mcp/src/handlers/run.ts:57-60).

## Architecture

Dispatch change (crates/cli/src/main.rs):

```rust
Some(Commands::Run(args)) => commands::run::run_firmware(args, cli.json),
```

`run_firmware` gains a `json: bool` parameter for structured diagnostics and
branches **before** any chip read:

- `args.system.is_some()` → `commands::run_system::run_firmware_with_system(&args, json)`.
- `args.system.is_none()` → existing per-arch dispatch with `--chip` unwrapped
  (the only mechanical edits in `run_firmware`: `args.chip` is read at
  crates/cli/src/commands/run.rs:506 and passed at :55/:68/:1119/:1130).

New module `crates/cli/src/commands/run_system.rs` drives:

1. Parse each `--stimulus` argument into `labwired_config::StimulusSpec`
   (absent/zero `after_cycles` → `FaultTrigger::AtStart`; non-zero →
   `FaultTrigger::AfterCycles`).
2. Parse the manifest with `SystemManifest::from_file`, resolve `manifest.chip`
   relative to the manifest, and parse the `ChipDescriptor`. Reject non-`arm`
   archs per the Arch Support table. (Do not call `build_system_bus`, which
   parses the manifest and chip and discards them — builder.rs:14-32; the
   driver needs the descriptor for arch selection.)
3. Build the bus with `SystemBus::from_config(&chip, &manifest)`.
4. Load the firmware ELF and construct the `Machine` exactly as
   `run_interactive_arm` does (`configure_cortex_m` + `Machine::load_firmware`).
5. Validate every spec against the bus before stepping (see Validation). No
   value is applied during validation, so `after_cycles` timing is unaffected.
6. Require `--max-steps` (validated in step 1's error pass).
7. Apply `at_start` specs, then run a batched loop using
   `Machine::advance` (`AdvanceRequest::run` with a 10 000-instruction cap and
   `BreakpointPolicy::Ignore`), clamped to `StimulusTrack::next_deadline_after(current_cycle)`
   and the remaining step budget — the same shape as the test loop
   (crates/cli/src/main.rs:1656-1664) minus limits/assertions. Due
   `after_cycles` specs fire at batch boundaries via
   `StimulusTrack::poll(machine, machine.total_cycles)`; `machine.total_cycles`
   is the canonical counter, so no observer installation is required (the
   `PerformanceMetrics` observer trap is avoided deliberately).
8. Export `--bus-trace-out` on completion via the existing
   `export_bus_trace_if_requested` helper. UART streaming, stop-reason mapping
   to exit codes, and the final stderr progress line follow the existing `run`
   paths; `--json` affects error payloads only.

New shared module `crates/cli/src/stimuli.rs`:

```rust
/// Deserialized `--stimulus` argument (agent-facing MCP schema).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliStimulus {
    pub channel: String,
    pub value: f64,
    #[serde(default)]
    pub after_cycles: Option<u64>,
    #[serde(default)]
    pub component: Option<String>,
}

pub fn parse_stimulus_arg(index: usize, raw: &str) -> Result<StimulusSpec, StimulusArgError>;

/// At-start application plus once-only `after_cycles` firing.
pub struct StimulusTrack { /* specs, fired flags */ }

impl StimulusTrack {
    pub fn new(specs: &[StimulusSpec]) -> Self;
    pub fn apply_at_start<C: Cpu>(&mut self, machine: &mut Machine<C>);
    /// Earliest unfired `after_cycles` threshold strictly after
    /// `current_cycle`; both loops clamp their batch to it. Past deadlines are
    /// filtered so a due-but-unfired spec cannot mask a later one when the
    /// poll cycle source lags.
    pub fn next_deadline_after(&self, current_cycle: u64) -> Option<u64>;
    pub fn poll<C: Cpu>(&mut self, machine: &mut Machine<C>, cycles: u64);
}
```

`execute_test_loop` (crates/cli/src/main.rs:1487-1511, 1601-1615, 1656-1666) is
refactored onto `StimulusTrack`. This refactor is behavior-preserving and must
keep the exact stderr strings `stimulus: {channel} = {value} applied` and the
current failure line, because e2e_kw41z_cow_stimulus.rs:119 asserts them. The
refactor lands as its own commit, gated by the existing stimulus e2e tests
(`e2e_kw41z_cow_stimulus`, `e2e_leo_airquality`), before the new driver uses
the helper.

## Core API Addition

`SystemBus::set_input` resolves `component` against
`si.component_id() == Some(c) || c == peripheral bus name`
(crates/core/src/bus/sim_inputs.rs:168-174), but both `component_matches` and
`for_each_sim_input` are `pub(crate)`, and `list_inputs` collapses identity to
one owner string (`component_id().unwrap_or(name)`, :182-192). CLI-side
validation therefore cannot reproduce resolution from public API: it would
reject the documented `component: "i2c1"` alias (crates/config/src/lib.rs:
1735-1750) and advertise an inventory that omits aliases.

Add to the core crate:

```rust
/// Read-only resolution: the same walk and uniqueness rules `set_input`
/// enforces, returning the channel metadata without applying a value.
pub fn resolve_input(
    &mut self,
    component: Option<&str>,
    channel: &str,
) -> Result<crate::sim_input::InputChannel, crate::sim_input::SimInputError>;
```

`set_input` is refactored to resolve through the same function so validation
and dispatch cannot drift. `resolve_input` exposes `InputChannel::{min,max}`,
which the CLI uses for the range check. Unit tests must cover the peripheral
bus-name alias (`i2c1`) and the ambiguous-channel case.

## Data Flow

```
argv
  → clap parses --system / --stimulus (raw strings)
  → parse_stimulus_arg per entry → Vec<StimulusSpec>          [fail: indexed]
  → SystemManifest::from_file → chip path → ChipDescriptor    [fail: not arm]
  → SystemBus::from_config → Machine (configure_cortex_m)
  → resolve_input per spec + [min,max] range check            [fail: inventory]
  → StimulusTrack::apply_at_start
  → loop: advance(batch clamped to next_deadline_after + budget)
          StimulusTrack::poll(machine, machine.total_cycles)
  → bus trace export, stderr report, exit-code mapping
```

## Error Handling

| Failure | Exit | Message / payload |
| --- | --- | --- |
| Malformed JSON, unknown field, missing `channel`/`value`, bad `after_cycles` | `EXIT_CONFIG_ERROR` | `--stimulus[i]: <detail>`; JSON payload under `--json` |
| Unknown channel | `EXIT_CONFIG_ERROR` | human message + `available: [{component, channel}]` inventory under `--json` |
| Ambiguous bare channel | `EXIT_CONFIG_ERROR` | same, plus candidate components for that channel |
| Out-of-range value | `EXIT_CONFIG_ERROR` | channel, accepted `[min, max]` and unit from `InputChannel` |
| `--stimulus` without `--system` | `EXIT_CONFIG_ERROR` | "stimuli target devices from a system manifest; pass --system" |
| `--system` without `--max-steps` | `EXIT_CONFIG_ERROR` | "run --system requires an explicit --max-steps budget" |
| `--gpio-trace` with `--system` | `EXIT_CONFIG_ERROR` | "gpio-trace is only supported on the ESP32-S3 run path" |
| Non-`arm` manifest | `EXIT_CONFIG_ERROR` | arch name + supported set (see Arch Support) |
| Set fails at fire time (passed pre-validation) | run continues | `error!` log line, matching current test-loop semantics |

Note on JSON number limits: `serde_json` rejects `NaN`/`Infinity` and
out-of-`f64`-range literals itself, and a negative `after_cycles` fails `u64`
deserialization. The `value.is_finite()` guard stays for parity with
`TestScript::validate` (crates/config/src/lib.rs:1859), but its unit test must
construct the struct directly rather than feed JSON.

## Testing

Unit tests:

- `crates/cli/src/stimuli.rs`: valid entity, missing/unknown fields,
  `after_cycles: 0` → `AtStart`, negative cycles rejected, index in error
  messages; `StimulusTrack` fires each spec once, `next_deadline_after` order.
- `crates/core` bus tests: `resolve_input` accepts the `i2c1` alias,
  rejects ambiguous/unknown, and `set_input` keeps identical results after
  sharing the resolution path.

Integration tests (`crates/cli/tests/e2e_run_stimulus.rs`), ARM only:

- KW41Z cow activity: `run --system configs/systems/frdm-kw41z-lcd.yaml
  --firmware tests/fixtures/kw41z-lcd-activity.elf --max-steps 6000000
  --stimulus '{"component":"fxos8700","channel":"x","value":2.0,"after_cycles":3000000}'`
  produces `MOOD=CALM` then `MOOD=ACTIVE` on stdout, and asserts the stderr
  line `stimulus: x = 2` — the same observables as the test-script run. A
  wall-clock budget is recorded in the test (batched loop, not single-step).
- Fail-fast: unknown channel exits non-zero; with `--json` the error lists
  the `fxos8700:x` drive point.
- Guard rails: `--stimulus` without `--system`; `--system` without
  `--max-steps`; `--gpio-trace` with `--system`; ESP32-WROOM manifest rejected
  with an arch message.
- Regression: `run` without `--system` unchanged (existing suite).

Refactor gates (before the new driver lands): `cargo test -p labwired-cli
--test e2e_kw41z_cow_stimulus -- --nocapture` and `--test e2e_leo_airquality`
pass unchanged.

## Validation Plan

From `core/`:

```console
cargo test -p labwired-core resolve_input
cargo test -p labwired-cli
cargo test -p labwired-cli --test e2e_run_stimulus -- --nocapture
cargo clippy -p labwired-cli -p labwired-core -- -D warnings
cargo fmt --all -- --check
```

## Documentation

- `--help` text for `--system` and `--stimulus` states the JSON shape, the
  `--stimulus` requires `--system` rule, and the `run --system` budget rule.
- The KW41Z e2e command is the canonical agent example; no marketing or
  website docs are in scope.
