# CI Test Runner (`labwired test`)

LabWired provides a CI-friendly runner mode driven by a YAML test script:

```bash
labwired test --script test.yaml
```

Every v1.0 script selects exactly one run shape: a single machine or an
environment world. The two `inputs` forms are mutually exclusive.

### Single-machine script

Use `inputs.firmware` (and optionally `inputs.system`) to run one machine. A
per-machine `system.yaml` is the same system manifest consumed by the
Playground; CI does not use a separate board-description format.

```bash
labwired test --firmware path/to/fw.elf --system system.yaml --script test.yaml
```

```yaml
schema_version: "1.0"
inputs:
  firmware: "relative/or/absolute/path/to/fw.elf"
  system: "optional/path/to/system.yaml"
limits:
  max_steps: 100000
assertions:
  - uart_contains: "Hello"
```

`--firmware` and `--system` are single-machine overrides only. They are not
valid with an environment script.

### Environment/world script

Use `inputs.env` to select an environment manifest. The manifest owns the
world topology: every node supplies its `id`, the shared-with-Playground
`system.yaml`, and its firmware ELF. Connectivity belongs only in explicit
`interconnects`; it is never implied by CLI overrides.

```yaml
# test.yaml
schema_version: "1.0"
inputs:
  env: "two-node-env.yaml"
limits:
  max_steps: 100000
  max_cycles: 123456     # optional
  max_uart_bytes: 4096   # optional
  wall_time_ms: 5000     # optional
  stop_when_assertions_pass: true          # optional; default false
  stop_when_assertions_pass_settle_steps: 1000  # optional; default 100000
  stop_when_assertions_pass_min_steps: 1000     # optional; default 0
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
      size: 8
      mask: 0xff
```

```yaml
# two-node-env.yaml
schema_version: "1.0"
name: "two-node-smoke"
nodes:
  - id: alpha
    system: "systems/alpha.yaml"
    firmware: "firmware/alpha.elf"
  - id: beta
    system: "systems/beta.yaml"
    firmware: "firmware/beta.elf"
interconnects:
  - type: can_bus
    nodes: [alpha, beta]
    config:
      peripheral: can1
```

An environment assertion must be a node-qualified `memory_value` assertion.
v0.19 environment worlds are Cortex-M-only: each resolved node chip must
resolve to `arch: arm` and explicitly declare `core: cortex-m*`. Its firmware
must be an `EM_ARM` ELF with a valid Cortex-M Thumb reset vector at
`flash.base + reset_vector_offset`: the initial stack pointer must land in RAM
and the Thumb-bit reset handler must land in flash. A node outside that contract
is rejected as a configuration error before a Cortex-M machine is constructed.

`nodes[].config_overrides` must be omitted. Every explicit occurrence,
including `{}` and `null`, is rejected in environment schema 1.0.

Interconnect membership and `config` are validated before the world starts.
Each `config` mapping is closed and type-checked:

- `uart_cross_link` requires exactly two unique, known nodes. Its only optional
  keys are non-empty strings `node_a_uart` and `node_b_uart` (each defaults to
  `uart2`).
- `can_bus` requires at least two unique, known nodes. Its only key is required
  non-empty string `config.peripheral`.
- `egress` requires exactly one known node. Its only keys are optional non-empty
  strings `uart` (default `usart2`), `transport` (`tcp`, `mqtt`, or `http`),
  `encoding` (`raw`, `ndjson-trace`, or `frames-json`), required non-empty
  string `url`, MQTT-only required non-empty string `topic`, and positive
  integer `buffer_max`. `topic` is invalid for TCP and HTTP.

Environment scripts do not accept single-machine firmware/system overrides or
topology-affecting CLI options.

`stop_when_assertions_pass` is opt-in for an environment world and has the
same durable completion contract as a single-machine run. Once every
node-qualified `memory_value` assertion passes at or after
`stop_when_assertions_pass_min_steps`, it must remain true for
`stop_when_assertions_pass_settle_steps` world rounds (default `100000`). A
regression restarts that window. A runtime failure or a same-round
`wall_time_ms`, `max_cycles`, or `max_uart_bytes` limit wins over completion.
`max_steps` remains the outer execution bound, so completion on its final
allowed world round reports `assertions_passed`; otherwise the result has
`stop_reason: assertions_passed`. The three fields are respectively a boolean
and non-negative integers; explicit YAML `null` is rejected.

## Exit Codes

| Code | Meaning | Notes |
|---:|---|---|
| `0` | Pass | All assertions passed, and any non-success stop reason was explicitly asserted (if applicable). |
| `1` | Assertion failure | Includes failed assertions, and hitting `wall_time_ms` / `max_uart_bytes` / `no_progress_steps` without asserting the matching `expected_stop_reason`. |
| `2` | Config/script error | Invalid YAML, unknown fields, unsupported schema_version, missing/invalid inputs/limits, or a safety guard (e.g. max-steps cap). |
| `3` | Simulation/runtime error | Runtime failure (e.g. `memory_violation`, `decode_error`) **unless** an `expected_stop_reason` assertion matches the stop reason. |

Exit-code precedence:
1) Any failed assertion ⇒ `1` (even if a runtime error also occurred)
2) `wall_time` / `max_uart_bytes` / `no_progress` stop without matching `expected_stop_reason` ⇒ `1`
3) Runtime error stop without matching `expected_stop_reason` ⇒ `3`
4) Otherwise ⇒ `0`

## Script Schema (v1.0)

```yaml
schema_version: "1.0"
inputs:
  firmware: "relative/or/absolute/path/to/fw.elf"
  system: "optional/path/to/system.yaml"
limits:
  max_steps: 100000
  max_cycles: 123456     # optional
  max_uart_bytes: 4096   # optional
  no_progress_steps: 500 # optional (PC unchanged for N steps)
  wall_time_ms: 5000   # optional
assertions:
  - uart_contains: "Hello"
  - uart_regex: "^Hello.*$"
  - expected_stop_reason: max_steps
```

Notes:
- Unknown fields are rejected (script parse/validation returns exit code `2`).
- Relative `inputs.firmware` / `inputs.system` paths are resolved relative to the directory containing the script file (not the current working directory).
- Relative `inputs.env` is resolved relative to the test script; each node's `system` and `firmware` paths are resolved relative to the environment manifest.
- For a single-machine script, CLI flags override script inputs:
  - `--firmware` overrides `inputs.firmware`
  - `--system` overrides `inputs.system`
- For an environment script, `inputs.env` is the only topology input; `--firmware` and `--system` are rejected.
- CLI flags override script limits:
  - `--max-steps` overrides `limits.max_steps`
  - `--max-cycles` overrides `limits.max_cycles`
  - `--max-uart-bytes` overrides `limits.max_uart_bytes`
  - `--detect-stuck` (alias: `--no-progress`) overrides `limits.no_progress_steps`
- `--breakpoint <addr>` (repeatable) stops the run when PC matches and sets `stop_reason: halt`.

The single-machine example above permits the documented single-machine
assertions. Environment scripts are stricter: they require at least one
node-qualified `memory_value` assertion and support `max_steps`, `max_cycles`,
`max_uart_bytes`, `wall_time_ms`, `stop_when_assertions_pass`,
`stop_when_assertions_pass_settle_steps`, and
`stop_when_assertions_pass_min_steps` limits. They reject
`limits.no_progress_steps` and single-machine-only controls such as
`--breakpoint`.

### Deprecated Legacy Schema (v1)

For backward compatibility, `schema_version: 1` is still accepted, but is deprecated and will be removed in a future release.

Legacy shape:

```yaml
schema_version: 1
firmware: "optional/path/to/fw.elf"   # optional (can be provided by --firmware)
system: "optional/path/to/system.yaml"
max_steps: 100000
wall_time_ms: 5000   # optional
assertions: []
```

## Stop Reasons

`expected_stop_reason` supports:
- `max_steps`
- `max_cycles`
- `max_uart_bytes`
- `no_progress`
- `wall_time`
- `assertions_passed`
- `memory_violation`
- `decode_error`
- `halt`
- `exception`
- `config_error` (runner failed before simulation started; e.g. script parse/validation error)

Semantics:
- If the simulator hits `wall_time_ms`, the run is treated as an assertion failure (exit code `1`) unless an `expected_stop_reason` assertion matches `wall_time`.
- If the simulator hits `max_uart_bytes` or `no_progress_steps`, the run is treated as an assertion failure (exit code `1`) unless an `expected_stop_reason` assertion matches (`max_uart_bytes` / `no_progress`).
- If the simulator hits `max_steps` or `max_cycles`, the run is considered a normal stop (exit code `0`) as long as assertions pass.
- If opt-in assertion completion reaches its durable settling window, the run
  stops with `assertions_passed` and passes.
- If the simulator hits a runtime error stop reason (e.g. `memory_violation`), the run is treated as a runtime error (exit code `3`) unless an `expected_stop_reason` assertion matches the stop reason.

`result.json` uses:
- `result_schema_version`: the result-union discriminator: `"1.0"` for a single machine or `"1.0-environment"` for an environment world
- `run_type: "environment"`: present only on the environment arm
- `stop_reason`: the terminal reason the simulator stopped
- `stop_reason_details`: which stop condition triggered (+ the limit/observed value when applicable)
- `limits`: the resolved limits used for the run (after applying any CLI overrides)
- `status`: one of `pass`, `fail`, `error`

## Artifacts

Use `--output-dir` to write artifacts:

```bash
labwired test --script test.yaml --output-dir out/artifacts
```

Artifacts:
- `out/artifacts/result.json`: machine-readable summary
- `out/artifacts/snapshot.json`: a CPU snapshot for a single machine, or an environment snapshot with a `nodes` array for a world (also written for config errors)
- `out/artifacts/uart.log`: captured UART TX bytes; world output is grouped by node
- `out/artifacts/junit.xml`: JUnit XML report (one testcase for `run` + one per assertion)

Alternatively, you can write JUnit XML to a specific path:

```bash
labwired test --script test.yaml --junit out/junit.xml
```

## Path Resolution Rules

- `--script`: if relative, resolved relative to the current working directory.
- Script-relative paths:
  - `inputs.firmware`, `inputs.system` (v1.0)
  - `inputs.env` (environment v1.0)
  - `firmware`, `system` (legacy v1)
  are resolved relative to the directory containing the script file.
- Environment-manifest-relative paths: each `nodes[].system` and
  `nodes[].firmware` value is resolved relative to the environment manifest.
- System manifest-relative paths: `system.yaml` may reference `chip: ...`; this `chip` path is resolved relative to the directory containing the system manifest file.
- `--output-dir` / `--junit`: if relative, resolved relative to the current working directory.

## `result.json` Contract

The runner writes `result.json` only when `--output-dir` is provided,
including config/script errors that exit with code `2`. Its top-level shape is
a discriminated union. Consumers must branch on
`result_schema_version`—and, for worlds, `run_type`—rather than assuming one
firmware identity.

### JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "LabWired CI runner result.json",
  "$defs": {
    "common": {
      "type": "object",
      "required": [
        "status",
        "steps_executed",
        "cycles",
        "instructions",
        "stop_reason",
        "stop_reason_details",
        "limits",
        "assertions"
      ],
      "properties": {
        "status": { "enum": ["pass", "fail", "error"] },
        "steps_executed": { "type": "integer", "minimum": 0 },
        "cycles": { "type": "integer", "minimum": 0 },
        "instructions": { "type": "integer", "minimum": 0 },
        "stop_reason": {
          "enum": [
            "config_error",
            "max_steps",
            "max_cycles",
            "max_uart_bytes",
            "no_progress",
            "wall_time",
            "assertions_passed",
            "memory_violation",
            "decode_error",
            "halt",
            "exception"
          ]
        },
        "stop_reason_details": { "type": "object" },
        "limits": { "type": "object" },
        "message": { "type": ["string", "null"] },
        "assertions": { "type": "array" },
        "fidelity": { "type": "array" }
      }
    },
    "environment_node": {
      "type": "object",
      "required": ["id", "system", "firmware", "system_hash", "firmware_hash"],
      "properties": {
        "id": { "type": "string" },
        "system": { "type": "string" },
        "firmware": { "type": "string" },
        "system_hash": { "type": "string" },
        "firmware_hash": { "type": "string" }
      }
    }
  },
  "oneOf": [
    {
      "title": "single-machine result",
      "allOf": [
        { "$ref": "#/$defs/common" },
        {
          "type": "object",
          "required": ["result_schema_version", "firmware_hash", "config"],
          "properties": {
            "result_schema_version": { "const": "1.0" },
            "firmware_hash": {
              "type": "string",
              "description": "SHA-256 of the firmware ELF bytes."
            },
            "config": {
              "type": "object",
              "required": ["firmware", "system", "script"],
              "properties": {
                "firmware": { "type": "string" },
                "system": { "type": ["string", "null"] },
                "script": { "type": "string" }
              }
            }
          }
        }
      ]
    },
    {
      "title": "environment/world result",
      "allOf": [
        { "$ref": "#/$defs/common" },
        {
          "type": "object",
          "required": ["result_schema_version", "run_type", "config"],
          "not": { "required": ["firmware_hash"] },
          "properties": {
            "result_schema_version": { "const": "1.0-environment" },
            "run_type": { "const": "environment" },
            "config": {
              "type": "object",
              "required": ["script", "environment", "world_firmware_hash", "nodes"],
              "properties": {
                "script": { "type": "string" },
                "environment": { "type": "string" },
                "world_firmware_hash": { "type": "string" },
                "nodes": {
                  "type": "array",
                  "items": { "$ref": "#/$defs/environment_node" }
                }
              }
            }
          }
        }
      ]
    }
  ]
}
```

The single-machine arm has the top-level `firmware_hash` and a
`config.firmware/system/script` provenance object. The environment arm has no
top-level `firmware_hash`: it identifies the world with
`config.world_firmware_hash` and records each node's `id`, `system`,
`firmware`, `system_hash`, and `firmware_hash`. A world config error still
uses the environment arm; its `nodes` list can be empty when the manifest
could not be resolved. That includes explicit `config_overrides` (including
`{}` and `null`), a chip without an explicit Cortex-M core, an ARM ELF without
a valid Thumb reset vector, and unknown, mistyped, or invalid interconnect
configuration; these produce `status: "error"` with `stop_reason:
"config_error"` without changing the result-union arm.

## CI release runners

Use the pinned v0.19.2 release runner in CI. It runs the same `labwired test`
command described above and writes the same artifact contract.

### GitHub Actions

Use the public Core action and pin the Core CLI with its version input. Its
only inputs are required `script`, optional `version` (default
`v0.19.2`), `output-dir`, and `args`:

~~~yaml
- id: labwired
  name: Run LabWired tests
  uses: w1ne/labwired-core/.github/actions/labwired-test@0cadd18fc9a3c0cbd1ecb0a6ddcd8ce66d56283d
  with:
    version: v0.19.2
    script: examples/ci/dummy-max-steps.yaml
    output-dir: out/artifacts
    args: --no-uart-stdout

- name: Link the automatic LabWired artifact
  if: always()
  run: echo "${{ steps.labwired.outputs.artifact-url }}" >> "$GITHUB_STEP_SUMMARY"
~~~

The Core action is an immutable action-source pin to
`0cadd18fc9a3c0cbd1ecb0a6ddcd8ce66d56283d`; `version: v0.19.2` independently
pins the immutable Core CLI release. It downloads that public release archive
with `curl`, creates `output-dir/junit.xml` plus Markdown and HTML reports,
appends the Markdown report to the job summary, and always uploads the entire
output directory—even after a failed test. Its `status`, `summary-md`,
`report-html`, `artifact-url`, and `exit-code` outputs are available through
the `labwired` step ID.

### Docker and GitLab runners

The GHCR image has labwired as its entrypoint. For Docker, pass test directly
after the pinned image name. Docker and the Action accept the same test YAML:
use either a single-machine script or an `inputs.env` world script.

~~~bash
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/workspace" -w /workspace \
  ghcr.io/w1ne/labwired:v0.19.2 \
  test --script examples/ci/dummy-max-steps.yaml \
       --output-dir out/artifacts \
       --no-uart-stdout
~~~

GitLab should clear that entrypoint and invoke labwired from its job shell:

~~~yaml
image:
  name: ghcr.io/w1ne/labwired:v0.19.2
  entrypoint: [""]
script:
  - labwired test --script examples/ci/dummy-max-steps.yaml --output-dir out/artifacts --no-uart-stdout
~~~

### Advanced source build

Build from source only when validating an unreleased LabWired commit. It is not
the normal CI path:

~~~bash
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/ci/dummy-max-steps.yaml --output-dir out/artifacts --no-uart-stdout
~~~
