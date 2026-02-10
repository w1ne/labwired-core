# CI Test Runner (`labwired test`)

LabWired provides a CI-friendly runner mode driven by a YAML test script:

```bash
labwired test --script test.yaml
```

You can override script inputs with CLI flags:

```bash
labwired test --firmware path/to/fw.elf --system system.yaml --script test.yaml
```

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
- CLI flags override script inputs:
  - `--firmware` overrides `inputs.firmware`
  - `--system` overrides `inputs.system`
- CLI flags override script limits:
  - `--max-steps` overrides `limits.max_steps`
  - `--max-cycles` overrides `limits.max_cycles`
  - `--max-uart-bytes` overrides `limits.max_uart_bytes`
  - `--detect-stuck` (alias: `--no-progress`) overrides `limits.no_progress_steps`
- `--breakpoint <addr>` (repeatable) stops the run when PC matches and sets `stop_reason: halt`.

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
- `memory_violation`
- `decode_error`
- `halt`
- `config_error` (runner failed before simulation started; e.g. script parse/validation error)

Semantics:
- If the simulator hits `wall_time_ms`, the run is treated as an assertion failure (exit code `1`) unless an `expected_stop_reason` assertion matches `wall_time`.
- If the simulator hits `max_uart_bytes` or `no_progress_steps`, the run is treated as an assertion failure (exit code `1`) unless an `expected_stop_reason` assertion matches (`max_uart_bytes` / `no_progress`).
- If the simulator hits `max_steps` or `max_cycles`, the run is considered a normal stop (exit code `0`) as long as assertions pass.
- If the simulator hits a runtime error stop reason (e.g. `memory_violation`), the run is treated as a runtime error (exit code `3`) unless an `expected_stop_reason` assertion matches the stop reason.

`result.json` uses:
- `result_schema_version`: version of the `result.json` contract (currently `"1.0"`)
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
- `out/artifacts/snapshot.json`: machine-readable snapshot of CPU state (or config error details)
- `out/artifacts/uart.log`: captured UART TX bytes
- `out/artifacts/junit.xml`: JUnit XML report (one testcase for `run` + one per assertion)

Alternatively, you can write JUnit XML to a specific path:

```bash
labwired test --script test.yaml --junit out/junit.xml
```

## Path Resolution Rules

- `--script`: if relative, resolved relative to the current working directory.
- Script-relative paths:
  - `inputs.firmware`, `inputs.system` (v1.0)
  - `firmware`, `system` (legacy v1)
  are resolved relative to the directory containing the script file.
- System manifest-relative paths: `system.yaml` may reference `chip: ...`; this `chip` path is resolved relative to the directory containing the system manifest file.
- `--output-dir` / `--junit`: if relative, resolved relative to the current working directory.

## `result.json` Contract (v1.0)

The runner writes `result.json` only when `--output-dir` is provided (including config/script errors that exit with code `2`).

### JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "LabWired CI runner result.json (v1.0)",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "result_schema_version",
    "status",
    "steps_executed",
    "cycles",
    "instructions",
    "stop_reason",
    "stop_reason_details",
    "limits",
    "assertions",
    "firmware_hash",
    "config"
  ],
  "properties": {
    "result_schema_version": {
      "type": "string",
      "enum": ["1.0"]
    },
    "status": {
      "type": "string",
      "enum": ["pass", "fail", "error"]
    },
    "steps_executed": { "type": "integer", "minimum": 0 },
    "cycles": { "type": "integer", "minimum": 0 },
    "instructions": { "type": "integer", "minimum": 0 },
    "stop_reason": {
      "type": "string",
      "enum": [
        "config_error",
        "max_steps",
        "max_cycles",
        "max_uart_bytes",
        "no_progress",
        "wall_time",
        "memory_violation",
        "decode_error",
        "halt"
      ]
    },
    "message": { "type": ["string", "null"] },
    "stop_reason_details": {
      "type": "object",
      "additionalProperties": false,
      "required": ["triggered_stop_condition", "triggered_limit", "observed"],
      "properties": {
        "triggered_stop_condition": {
          "type": "string",
          "enum": [
            "config_error",
            "max_steps",
            "max_cycles",
            "max_uart_bytes",
            "no_progress",
            "wall_time",
            "memory_violation",
            "decode_error",
            "halt"
          ]
        },
        "triggered_limit": {
          "type": ["object", "null"],
          "additionalProperties": false,
          "required": ["name", "value"],
          "properties": {
            "name": { "type": "string" },
            "value": { "type": "integer", "minimum": 0 }
          }
        },
        "observed": {
          "type": ["object", "null"],
          "additionalProperties": false,
          "required": ["name", "value"],
          "properties": {
            "name": { "type": "string" },
            "value": { "type": "integer", "minimum": 0 }
          }
        }
      }
    },
    "limits": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "max_steps",
        "max_cycles",
        "max_uart_bytes",
        "no_progress_steps",
        "wall_time_ms"
      ],
      "properties": {
        "max_steps": { "type": "integer", "minimum": 0 },
        "max_cycles": { "type": ["integer", "null"], "minimum": 0 },
        "max_uart_bytes": { "type": ["integer", "null"], "minimum": 0 },
        "no_progress_steps": { "type": ["integer", "null"], "minimum": 0 },
        "wall_time_ms": { "type": ["integer", "null"], "minimum": 0 }
      }
    },
    "message": {
      "type": "string",
      "description": "Present only for config errors / invalid inputs."
    },
    "assertions": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["assertion", "passed"],
        "properties": {
          "passed": { "type": "boolean" },
          "assertion": {
            "oneOf": [
              {
                "type": "object",
                "additionalProperties": false,
                "required": ["uart_contains"],
                "properties": { "uart_contains": { "type": "string" } }
              },
              {
                "type": "object",
                "additionalProperties": false,
                "required": ["uart_regex"],
                "properties": { "uart_regex": { "type": "string" } }
              },
              {
                "type": "object",
                "additionalProperties": false,
                "required": ["expected_stop_reason"],
                "properties": {
                  "expected_stop_reason": {
                    "type": "string",
                    "enum": [
                      "max_steps",
                      "max_cycles",
                      "max_uart_bytes",
                      "no_progress",
                      "wall_time",
                      "memory_violation",
                      "decode_error",
                      "halt"
                    ]
                  }
                }
              }
            ]
          }
        }
      }
    },
    "firmware_hash": {
      "type": "string",
      "description": "SHA-256 of the firmware ELF bytes (lowercase hex).",
      "pattern": "^[0-9a-f]{64}$"
    },
    "config": {
      "type": "object",
      "additionalProperties": false,
      "required": ["firmware", "system", "script"],
      "properties": {
        "firmware": { "type": "string" },
        "system": { "type": ["string", "null"] },
        "script": { "type": "string" }
      }
    }
  }
}
```

## GitHub Actions Example

```yaml
- name: Run LabWired tests
  run: |
    cargo build --release -p labwired-cli
    ./target/release/labwired test \
      --script examples/ci/dummy-max-steps.yaml \
      --output-dir out/artifacts \
      --no-uart-stdout
- name: Upload artifacts (pass/fail)
  if: always()
  uses: actions/upload-artifact@v4
  with:
    name: labwired-artifacts
    path: out/artifacts
    if-no-files-found: warn
```

## Copy-Paste Workflow (Composite Action)

This repo includes a minimal composite action wrapper at `.github/actions/labwired-test` that:
- builds `labwired` (`crates/cli`)
- runs `labwired test`
- emits artifact paths as outputs
- writes a small summary into the GitHub Actions step summary

Copy-paste this workflow into `.github/workflows/labwired-test.yml`:

```yaml
name: LabWired CI Test

on:
  pull_request:
  push:
    branches: [ "main", "develop" ]

jobs:
  labwired-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Run labwired test
        id: labwired
        uses: ./.github/actions/labwired-test
        with:
          script: examples/ci/dummy-max-steps.yaml
          output_dir: out/artifacts
          no_uart_stdout: true
          profile: release

      - name: Upload artifacts (pass/fail)
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: labwired-artifacts
          path: ${{ steps.labwired.outputs.artifacts_dir }}
          if-no-files-found: warn

      - name: Fail job if test failed
        if: ${{ steps.labwired.outputs.exit_code != '0' }}
        run: |
          echo "labwired test failed with exit_code=${{ steps.labwired.outputs.exit_code }}"
          exit ${{ steps.labwired.outputs.exit_code }}
```

## Local vs CI Parity

CI runs the same `labwired test` command you can run locally; the only CI-specific behavior is how artifacts are uploaded and how the summary is displayed.

Local (native):

```bash
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/ci/dummy-max-steps.yaml --output-dir out/artifacts --no-uart-stdout
```

Local (Docker, closest to “clean CI machine”):

```bash
docker build -t labwired-ci .
docker run --rm -v "$PWD:/work" -w /work labwired-ci \
  labwired test --script examples/ci/dummy-max-steps.yaml --output-dir out/artifacts --no-uart-stdout
```
