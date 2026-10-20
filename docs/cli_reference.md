---
description: "The labwired command-line interface is the primary entry point for running simulations, testing, debugging, and managing assets."
---

# CLI Reference

The `labwired` command-line interface is the primary entry point for running simulations, testing, debugging, and managing assets. This page documents every command listed by `labwired --help`. The built-in help is authoritative when the two disagree; run `labwired <command> --help` for the version you have.

## Global Options

These flags are declared as global options, so every subcommand accepts them. The per-command tables below list only that command's own flags.

| Option | Description |
| :--- | :--- |
| `-t, --trace` | Enable instruction-level execution tracing. `test` writes `trace.json` into `--output-dir`. |
| `--json` | Output errors and diagnostics as structured JSON for agent consumption. |
| `--vcd <PATH>` | Output a Value Change Dump (VCD) trace file to the specified path. |
| `--rtt` | Emit SEGGER RTT output: interactive runs echo drained up-channel bytes and write stdin into down-channel 0; `test` writes `rtt.log` and enables `rtt_contains`. |
| `--semihosting` | Emit ARM semihosting output: interactive runs echo drained bytes to stdout; `test` writes `semihosting.log` and enables `semihosting_contains`. |
| `--itm` | Emit ITM stimulus port 0: interactive runs echo the byte stream to stdout; `test` writes `itm.log` and enables `itm_contains`. |
| `-V, --version` | Print version information. |
| `-h, --help` | Print help. `labwired help [COMMAND]` prints the same help for a subcommand. |

## Modes & Commands

### Interactive / Run Mode (Default)

Running `labwired` with no subcommand starts the interactive runner:

```bash
labwired --firmware <ELF> --system <YAML> [OPTIONS]
```

| Option | Description |
| :--- | :--- |
| `-f, --firmware <PATH>` | Path to the firmware ELF file. |
| `-s, --system <PATH>` | Path to the system manifest (YAML). |
| `--snapshot <PATH>` | Write a JSON state snapshot on exit; resume it with `labwired machine load --snapshot <PATH>`. |
| `--breakpoint <ADDR>` | Breakpoint PC address (decimal or `0x` hex). Repeatable. |
| `--max-steps <N>` | Maximum number of steps to execute (default: 20000). |
| `--gdb <PORT>` | Start a GDB RSP server on the specified port (e.g. `3333`). |

Plus the [global flags](#global-options). The top-level `--max-steps` default (20000) is separate from `run --max-steps`, which defaults to unlimited.

#### `--gdb` — GDB RSP server

Start the server with `labwired --firmware app.elf --system board.yaml --gdb 3333`. The RSP surface is documented in [GDB Integration](gdb_integration.md#3-supported-commands); two debugger behaviours worth knowing:

- **Watchpoints** — `Z2`/`Z3`/`Z4` arm a write/read/access watch on a byte range of `1..=4096` bytes, up to 16 concurrent ranges. They are software-tracked on CPU accesses only: DMA and debugger transfers (`m`/`M`/`x`) do not trip them, so inspecting a watched object cannot fire its own watchpoint. A hit stops the target with a bare `T05`/`S05` (no `watch:` annotation; read `PC` back with `g`). Details: [Watchpoints](gdb_integration.md#5-watchpoints-z2-z3-z4).
- **`P`** — writes a single register for real (`reg=value` form), so `set $pc = ...` and `reset $pc` take effect; see the CPSR raw-id quirk in the same section.

Which clients have actually run against the stub is tracked in the [conformance matrix](gdb_integration.md#6-conformance-matrix); every cell is either CI-backed or marked `untested`.

### `run`

Run a firmware ELF in the simulator using a chip descriptor. The chip's peripheral wiring is loaded, the firmware is fast-booted, and console output appears on stdout in real time.

```bash
labwired run --chip <CHIP> --firmware <ELF> [OPTIONS]
```

| Option | Description |
| :--- | :--- |
| `--chip <PATH>` | Chip descriptor YAML. Required unless `--system` is given (the manifest then names the chip). |
| `--firmware <PATH>` | Path to the firmware ELF. Required. |
| `--system <PATH>` | Board manifest. With `--chip`, attaches its `external_devices:` before the run (ESP32-S3 only). Without `--chip`, selects the system-aware driver, which resolves the chip from the manifest, attaches external devices, and applies `--stimulus` (ARM only); an explicit `--max-steps` is then required. |
| `--stimulus <JSON>` | Declarative input stimulus, e.g. `{"channel":"x","value":2.0,"after_cycles":3000000,"component":"fxos8700"}`. Repeatable. Requires `--system` without `--chip`. |
| `--display-out <PATH>` | End-of-run dump of every attached parallel panel: binary PPM here plus a luma ASCII map on stderr. |
| `--stop-on <TEXT>` | End the run when the text appears on the console. ESP32-S3 only. See [`--stop-on`](#-stop-on-run) below. |
| `--max-steps <N>` | Step budget (default: unlimited; required with `--system` without `--chip`). Coalesced idle cycles count against it on accelerated ARM runs. |
| `--allow-sim-error` | Exit 0 even when the run ends on a simulation fault (normally exit 3). |
| `--gpio-trace <PATH>` | JSON-line GPIO transition trace: `{"sim_cycle":N,"pin":P,"from":B,"to":B}`. |
| `--gpio-trace-format <MODE>` | `bool` (default) or `4state`; the latter adds `from_state`/`to_state` (`0`, `1`, `z`, `x`) and logs drive changes too. |
| `--bus-trace-out <PATH>` | Export the I²C/SPI/UART bus trace. `.json` writes raw events; any other extension writes VCD. See [`--bus-trace-out`](#-bus-trace-out-run) below. |
| `--analog-trace <PATH>` | In-core analog waveform trace from an `adapter: analog` co-simulation model. `.csv` writes `time_ns,<channel>...`; other extensions write VCD `real` variables. Empty unless a co-sim runner is attached. |
| `--rom-boot` | Boot from the real ROM reset vector (`0x40000400`) instead of fast-booting. ESP32-S3; requires `LABWIRED_ESP32S3_FLASH` (ROM auto-provisioned from the ESP toolchain or pinned with `LABWIRED_ESP32S3_ROM`/`_DROM`/`_ROM_ELF`). |
| `--break-at <HEX>` | Debug: PC address(es) to dump registers and `--watch-mem` words at first hit, then continue. Repeatable. |
| `--watch-mem <HEX>` | Debug: memory address(es) printed as `u32` whenever a `--break-at` fires. Repeatable. |
| `--flash-image <PATH@HEX>` | ARM-only additional flash piece at an absolute address. Repeatable; ELF, Intel HEX, or raw binary. Overlaps are a hard error. |
| `--batched` | Drive the run through the batched orchestration instead of per-instruction `step()`. Fails rather than falls back when per-instruction instrumentation (`--break-at`, `--watch-mem`, `--stop-on`; on RISC-V also the WiFi bridge and DHCP trace) is active. ARM, RISC-V and Xtensa (ESP32 / ESP32-S3) all have batched paths. Prints a `[batched] ...` summary. |
| `--time-mode <MODE>` | `max-speed` (default, never sleeps) or `realtime` (sleeps when virtual time is ≥ 1 ms ahead of wall). |
| `--log-timestamps <MODE>` | `none` (default), `virtual`, `real`, or `both`. See [`--log-timestamps`](#-log-timestamps-run-and-test) below. |

Plus the [global flags](#global-options).

#### `--log-timestamps` (run and test)

`--log-timestamps {none,virtual,real,both}` prefixes the human-readable stderr event/log lines of a run — the `labwired-cli ...` progress and verdict lines and the tracing subscriber's event lines. The prefix is a fixed-format bracket:

| Value | Prefix | Meaning |
| :--- | :--- | :--- |
| `none` | nothing | Default. Every output stays byte-identical to a run without the flag. |
| `virtual` | `[t=virt=1.500000000s] ` | Simulation time from `cycles / cpu_hz`. Deterministic: two runs of the same firmware compare equal. |
| `real` | `[t=real=+0.123456789s] ` | Wall-clock since process start. Explicitly **not** comparable between runs. |
| `both` | `virtual` and `real` in one bracket | Both numbers. |

With `both`, the bracket joins the two numbers:

```text
[t=virt=1.500000000s|real=+0.123456789s] ...
```

With a prefixed mode active, the tracing formatter drops its own RFC 3339 wall-clock stamp so the `virtual` stream is byte-stable. `result.json` gains `virtual_time_ns` for `virtual`/`both` and `host_time_ns` for `real`/`both`; default `none` adds neither key and leaves the file unchanged.

Receipt — `test` over the committed UART fixture, with a two-input script whose paths are absolute (`inputs` resolve relative to the script file):

```yaml
schema_version: "1.0"
inputs:
  firmware: "/path/to/tests/fixtures/uart-ok-thumbv7m.elf"
  system: "/path/to/configs/systems/ci-fixture-uart1.yaml"
limits:
  max_steps: 1000
assertions: []
```

```bash
labwired test --script /tmp/cli-ref/logts/script.yaml \
  --no-uart-stdout --output-dir /tmp/cli-ref/logts/out \
  --log-timestamps virtual
```

Real output (trimmed — the full run also prints the ELF entry-point and `idle_ff` lines; wall-seconds vary between runs):

```text
[t=virt=0.000000000s]  INFO labwired_core::system::builder: Loading chip descriptor: ../chips/ci-fixture-cortex-m3-uart1.yaml
[t=virt=0.000000000s] labwired-cli test: hosted fast path (exact windows up to 1024 cycles)
[t=virt=0.000012500s] PASS  0/0 checks · script · 1000 steps · 0.00s
```

The same run's `result.json` carries `"virtual_time_ns": 12500` and no `host_time_ns` key.

#### `--bus-trace-out` (run)

`--bus-trace-out <PATH>` exports the universal I²C/SPI bus trace captured during the run. A `.json` extension writes the raw event list (each event has `seq`, `cycle`, `bus`, and the native payload); any other extension writes a VCD that opens directly in GTKWave / PulseView / sigrok. UART octets are recorded too, even though the flag's help names I²C/SPI.

Receipt — the committed C3 blinky, 57 UART octets:

```bash
labwired run --chip configs/chips/esp32c3.yaml \
  --firmware examples/esp32c3-blinky/firmware/esp32c3_blinky.elf \
  --max-steps 800000 --bus-trace-out /tmp/cli-ref/blinky.vcd
```

```text
LED Olabwired-cli run: bus trace (57 events) -> "/tmp/cli-ref/blinky.vcd"
```

The VCD declares one 8-bit wire (UART octets) and stamps each event at its sequence number with a 1 µs timescale:

```text
$timescale 1 us $end
$scope module bus_trace $end
$var wire 8 ! uart0 $end
$upscope $end
$enddefinitions $end
#1
b01000011 !
```

`b01000011` is `0x43` (`C`), the first byte of the firmware banner. Point the same flag at `blinky.json` for the lossless event list (`seq` 1, `cycle` 13312, `bus` `uart0`, payload `{"protocol":"uart","direction":"tx","byte":67}`). More on the ring and formats: [Hook surfaces](howto/extend/hooks.md#2-host-observation-bus-trace-console-stop-logic-capture-python-frames).

#### `--stop-on` (run)

`--stop-on <TEXT>` (ESP32-S3 only; it needs the USB-Serial-JTAG console) ends the run as soon as the text appears on the firmware's console. The console is mirrored into a buffer and scanned every 100,000 steps; the first match logs the step and breaks the loop. It cannot be combined with `--batched`.

Receipt — the committed S3 Arduino source-debug fixture; `81` is the fifth `loop()` print (one interleaved GPIO `INFO` line is elided between the console byte and the match line):

```bash
LABWIRED_ESP32S3_FLASH=tests/fixtures/source-debug/esp32s3-arduino-flash.bin \
  labwired run --chip configs/chips/esp32s3.yaml \
  --firmware tests/fixtures/source-debug/esp32s3-arduino.elf \
  --rom-boot --max-steps 200000000 --stop-on 81
```

```text
81labwired-cli run: --stop-on "81" matched at step 101700000
```

Two caveats:

- The `reached --max-steps` epilogue is printed only when the stepping loop actually exhausts the budget; a run stopped by `--stop-on` prints only the match line.
- A marker that occurs in boot output matches during boot. `--stop-on 27` matches the ROM banner `ESP-ROM:esp32s3-20210327` at step 100000, not application output:

```text
ESP-ROM:esp32s3-20210327
ESP-ROM:1
labwired-cli run: --stop-on "27" matched at step 100000
```

Pick a marker unique to the point you care about. The hook surface and the shared observation story are in [Hook surfaces](howto/extend/hooks.md).

### `test`

Runs a CI-friendly test script with assertions. Script schema, exit codes, stop reasons, and the `result.json` contract live in [CI Test Runner](ci_test_runner.md).

```bash
labwired test --script <YAML> [OVERRIDES]
```

| Option | Description |
| :--- | :--- |
| `-f, --firmware <PATH>` | Override the firmware in a single-machine script only; environment topology comes from `inputs.env`. |
| `-s, --system <PATH>` | Override the system manifest in a single-machine script only; environment topology comes from `inputs.env`. |
| `-c, --script <PATH>` | Path to the test script (YAML). Required. |
| `--max-steps <N>` | Override the script's step limit. |
| `--breakpoint <ADDR>` | Breakpoint PC address. Repeatable. |
| `--no-uart-stdout` | Disable UART stdout echo (still captured for assertions/artifacts). |
| `--output-dir <PATH>` | Artifact directory: `result.json`, `snapshot.json`, `uart.log`, `rtt.log`, `semihosting.log`, `itm.log`, `junit.xml`; requested `trace.json`, coverage, run-manifest, and fault artifacts land here too. |
| `--junit <PATH>` | Also write the JUnit XML report to this path. |
| `--max-cycles <N>` | Override the cycle limit. |
| `--max-uart-bytes <N>` | Override the UART output limit. |
| `--detect-stuck <N>` | Fail if the PC does not change for N steps (hang detection). Hidden alias: `--no-progress`. |
| `--max-vcd-bytes <N>` | Override the VCD size limit (bytes). |
| `--analog-trace <PATH>` | Analog waveform trace (`.csv` or VCD `real` variables). |
| `--trace-max <N>` | Maximum number of instructions to trace. |
| `--coverage` | Collect line/branch/function coverage from the unmodified ELF into `coverage.info` (LCOV), `coverage.cobertura.xml`, and `coverage.json`. See [Firmware coverage](howto/firmware-coverage.md). |
| `--rom-boot` | Faithful ESP32-C3 boot (`mask ROM → 2nd-stage bootloader → app`); requires `LABWIRED_ESP32C3_FLASH` with bootloader@`0x0`, partition table@`0x8000`, app@`0x10000`. |
| `--run-manifest` | Write `run-manifest.json` (input hashes, engine version, result subset, coverage summary, wall-clock-free digest). |
| `--capture-app-entry <PATH>` | Faithful rom-boot: snapshot the machine at application entry into a `.lwrs` blob while the run continues to `--max-steps`. |
| `--resume-snapshot <PATH>` | Resume from a `.lwrs` snapshot instead of cold-booting, after validating its chip + firmware self-key against the current inputs. |
| `--no-key` | Explicitly do not send `LABWIRED_API_KEY`, even when it is set. |
| `--watch-gpio <PERIPHERAL:PIN>` | Logic-analyzer edge capture for a pad, e.g. `gpio8:8`. Repeatable; channels land in `result.json`'s `logic_edges` block. |
| `--heartbeat-file <PATH>` | Supervisor liveness file, rewritten atomically at most once a second with `{"steps":N,"cycles":C}`. |
| `--log-timestamps <MODE>` | `none`, `virtual`, `real`, or `both`. See [`--log-timestamps`](#-log-timestamps-run-and-test) above. |

Plus the [global flags](#global-options), including `-t/--trace` (saved to `trace.json`) and `--vcd`.

### `coverage`

Report ESP32-S3 register-level peripheral coverage against the SVD: every register is probed behaviourally (read/write sentinel) and classified Modelled / Indeterminate / Unmodelled. Prints a human-readable table.

| Option | Description |
| :--- | :--- |
| `--svd <PATH>` | ESP32-S3 SVD; otherwise auto-discovered from PlatformIO or `LABWIRED_ESP32S3_SVD`. |
| `--json-out <PATH>` | Write the full coverage matrix as JSON. |

Plus the [global flags](#global-options).

#### `coverage merge`

Merge firmware-coverage LCOV files (from `test --coverage`, gcov/gcovr, llvm-cov, …) into one report: counts add up, and a branch stays "never evaluated" only if no run evaluated it. Prints a summary, writes LCOV / Cobertura / JSON, and can fail the build under a threshold. Worked example and CI setup: [Firmware coverage](howto/firmware-coverage.md#merging).

| Option | Description |
| :--- | :--- |
| `<INPUT>...` | LCOV files, or directories searched recursively for `*.info`. Earlier merge results and the `-o` target are skipped. |
| `-o, --output <PATH>` | Merged LCOV tracefile. |
| `--cobertura <PATH>` | Cobertura XML for GitLab / Jenkins / Azure DevOps. |
| `--source-root <DIR>` | Cobertura `<source>` base; repeatable, default `.`. |
| `--json-out <PATH>` | Summary, per-file numbers, and every uncovered branch as JSON. |
| `--fail-under-lines <PCT>` | Exit 1 below this line-coverage percentage. |
| `--fail-under-branches <PCT>` | Exit 1 below this branch-coverage percentage. |

Plus the [global flags](#global-options).

### `chips`

List the chips bundled with the CLI, usable as `inputs.chip` in a test script or as a manifest's `chip:` field without copying any YAML. No command-specific flags; accepts the [global flags](#global-options).

### `asset`

Utilities for Asset Foundry.

#### `asset import-svd`

Import an SVD file and convert it to Strict IR (JSON).

| Option | Description |
| :--- | :--- |
| `-i, --input <SVD>` | Input SVD file. |
| `-o, --output <JSON>` | Output JSON file. |
| `--flash-base <ADDR>` | Optional flash base address. |
| `--flash-size <SIZE>` | Optional flash size, e.g. `512KB`. |
| `--ram-base <ADDR>` | Optional RAM base address. |
| `--ram-size <SIZE>` | Optional RAM size, e.g. `128KB`. |

#### `asset codegen`

Generate Rust code from Strict IR.

| Option | Description |
| :--- | :--- |
| `-i, --input <JSON>` | Input Strict IR file. |
| `-o, --output <RS>` | Output Rust source file. |

#### `asset init`

Initialize a new project skeleton.

| Option | Description |
| :--- | :--- |
| `-o, --output <DIR>` | Output directory. Required. |
| `-c, --chip <CHIP>` | Chip name or path to a chip descriptor. |

#### `asset add-peripheral`

Add a peripheral to an existing chip descriptor.

| Option | Description |
| :--- | :--- |
| `-c, --chip <YAML>` | Target chip descriptor. |
| `-i, --id <NAME>` | New peripheral ID. |
| `--type <TYPE>` | Peripheral type (default `strict_ir`). |
| `-b, --base <ADDR>` | Base memory address. |
| `--ir-path <PATH>` | Path to the IR descriptor (JSON). |

#### `asset validate`

Validate a System Manifest and its referenced Chip.

| Option | Description |
| :--- | :--- |
| `--system <YAML>` | System manifest to validate. |
| `--chip <YAML>` | Chip descriptor to validate. |

#### `asset list-chips`

List available chip descriptors.

| Option | Description |
| :--- | :--- |
| `-f, --filter <TEXT>` | Filter string for chip names. |
| `--format <FORMAT>` | Output format: `text` (default) or `json`. |

#### `asset validate-component`

Validate an off-chip component IR spec (YAML).

| Option | Description |
| :--- | :--- |
| `<SPEC>` | Path to the device descriptor YAML. |
| `--json` | Emit machine-readable JSON on stdout. |

#### `asset ingest-svd`

Ingest an SVD into runnable declarative PeripheralDescriptor YAML — `type: declarative` peripherals, no codegen or recompile (unlike `import-svd`).

| Option | Description |
| :--- | :--- |
| `-i, --input <SVD>` | Input SVD file. |
| `-o, --output-dir <DIR>` | Directory to write `<peripheral>.yaml` descriptors into. |
| `--filter <NAMES>` | Only ingest these peripherals (comma-separated). Default: all. |
| `--json` | Emit a machine-readable JSON summary on stdout. |

### `machine`

Machine state control operations.

#### `machine load`

Load a machine state from a JSON snapshot and resume simulation.

| Option | Description |
| :--- | :--- |
| `-s, --snapshot <JSON>` | Path to the snapshot file. Required. |
| `--max-steps <N>` | Override the maximum number of steps to execute. |

Plus the [global flags](#global-options).

### `snapshot`

Capture a binary runtime snapshot (`.lwrs`) mid-flight for fast replay in the playground, restorable by `WasmSimulator::apply_runtime_snapshot`.

#### `snapshot capture`

Boot a firmware, step N times, write a runtime snapshot blob.

| Option | Description |
| :--- | :--- |
| `--firmware <ELF>` | Path to the firmware ELF. Required. |
| `--steps <N>` | Number of cycles to run before taking the snapshot. Required. |
| `--output <PATH>` | Output `.lwrs` path. Required. |
| `--system <YAML>` | Board manifest declaring external peripherals to attach. |
| `--profile <NAME>` | Firmware profile (default and only supported value: `arduino-esp32`). |
| `--progress-every <N>` | Progress line every N steps; `0` = silent (default `5000000`). |
| `--trace-out <PATH>` | JSON instruction trace of the last `--trace-last` retired instructions; forces the interpreter. |
| `--trace-last <N>` | Trace ring size (default `4096`). |

Plus the [global flags](#global-options).

### `debug-probe`

One-shot agent debug probe: loads machine + firmware, resolves optional breakpoints (address / symbol / line), runs until stop or `--max-steps`, and returns stop reason, PC, location, registers, and serial as JSON. Observational only — `proven` is always false.

| Option | Description |
| :--- | :--- |
| `-f, --firmware <ELF>` | Firmware ELF (required in v1; there is no flash-only rom-boot path). |
| `-s, --system <YAML>` | System manifest. Required. |
| `--breakpoints-json <JSON>` | JSON array of breakpoint specs. |
| `--breakpoints-file <PATH>` | Breakpoints JSON file (alternative to `--breakpoints-json`). |
| `--max-steps <N>` | Step budget (default `2000000`). |
| `--read <LIST>` | Comma list: `serial,regs,pc,location,fault` (default all). `fault` is present only when the firmware faulted. |
| `--step-after-stop` | After the primary stop, take one single-instruction step. |
| `--output-dir <DIR>` | Write `result.json` here (also prints JSON to stdout). |

Plus the [global flags](#global-options).

### `fault-inject`

Inject scheduled faults (register/memory bit flips, instruction skips) and report the lockstep verdict against a golden run as JSON. Verdicts: `masked`, `latent`, `diverged`, `output_changed`, `crashed`, `not_injected`.

| Option | Description |
| :--- | :--- |
| `-f, --firmware <ELF>` | Firmware ELF. Required. |
| `-s, --system <YAML>` | System manifest; its `chip:` is a built-in name or a path. Required. |
| `--plan <JSON>` | Plan file `{"until_cycle": N, "faults": [...]}`. Conflicts with `--faults-json`/`--until-cycle`. |
| `--faults-json <JSON>` | Faults as a JSON array, e.g. `[{"at_cycle":500,"kind":"register_bit_flip","register":"R0","bit":3}]`. Requires `--until-cycle`. Kinds: `register_bit_flip`, `memory_bit_flip`, `instruction_skip`. |
| `--until-cycle <N>` | Stop when the golden machine reaches this cycle. |
| `--output-dir <DIR>` | Also write the report to `<dir>/fault-report.json`. |

Plus the [global flags](#global-options).

### `fuzz`

Coverage-guided fuzzing in the simulator: mutates an input byte stream injected into the firmware's RAM buffer, drives execution with AFL-style edge coverage, and reports crashes. Exits non-zero when a crash is found.

| Option | Description |
| :--- | :--- |
| `--chip <YAML>` | Chip descriptor. Required. |
| `--system <YAML>` | System manifest. Required. |
| `--firmware <ELF>` | Firmware ELF following the fuzz contract. Required. |
| `--max-iters <N>` | Max fuzzing iterations (default `200000`). |
| `--max-steps <N>` | Max steps per run; past it is a hang (default `1000000`). |
| `--seed <N>` | RNG seed (default `3735928559`); deterministic for a fixed seed. |
| `--seed-input <HEX>` | Seed input as hex bytes, e.g. `5000`. Repeatable. |
| `--collect <N>` | Collect up to N distinct crashes instead of stopping at the first. |
| `--crashes-out <PATH>` | Write crashing inputs as a JSON array of byte arrays. |
| `--input-len-addr <ADDR>` | Contract: `u32` input-length word (default `0x20002800`). |
| `--input-data-addr <ADDR>` | Contract: input data buffer (default `0x20002804`). |
| `--verdict-addr <ADDR>` | Contract: `u32` verdict word (default `0x20003000`). |
| `--done-magic <HEX>` | Clean-completion verdict (default `0xC0DEF022`). |
| `--fault-magic <HEX>` | Fault/panic verdict (default `0xDEADFA17`). |

Plus the [global flags](#global-options).

### `tier1-matrix`

Run the Tier-1 chip × peripheral validation matrix and export it.

| Option | Description |
| :--- | :--- |
| `--json-out <PATH>` | Write the matrix as JSON (the committed snapshot is `docs/coverage/tier1-matrix.json`). |
| `--run-url <URL>` | Evidence link stamped into every cell that carries evidence (skips `na` and unrecorded). |

Plus the [global flags](#global-options).

### `cosim-step`

Step a manifest-declared co-simulation model through the real runner/adapter chain and print the routed outputs.

| Option | Description |
| :--- | :--- |
| `<SYSTEM>` | System manifest declaring `cosim_models`. |
| `--set <PATH=VALUE>` | Seed a signal-store path before stepping, e.g. `--set plant.channels.11.enabled=false`. Repeatable. |
| `--steps <N>` | Model steps to advance (default `1`). |
| `--json` | Print routed outputs as JSON instead of a table. |
| `--analog-trace <PATH>` | Analog waveform trace (`.csv` or VCD `real` variables); only `adapter: analog` models produce samples. |

Plus the [global flags](#global-options).

### `rv1103-linux`

Reduced RV1103 Linux direct boot with strict artifacts and a guest acceptance receipt.

| Option | Description |
| :--- | :--- |
| `--artifacts <DIR>` | Directory produced by `scripts/prepare-rv1103-linux.py`. Required. |
| `--receipt <PATH>` | Full bounded console, artifact/backend hashes, and acceptance evidence. Required. |
| `--cpu-hz <N>` | Functional instruction tick calibration (default `1200000000`; not physical timing). |
| `--max-cycles <N>` | Default `24000000000`. |
| `--max-retirements <N>` | Default `2000000000`. |
| `--max-fuel <N>` | Machine fuel, also counting idle fast-forward skips (default `24000000000`). |
| `--wall-seconds <N>` | Default `1800`. |
| `--console-bytes <N>` | UART2 byte cap, at most 64 KiB (default `65536`). |

Plus the [global flags](#global-options).

### `rv1103-basket`

Strict canonical smart-basket Linux and persistent power-plant local acceptance.

| Option | Description |
| :--- | :--- |
| `--artifacts <DIR>` | Required. |
| `--receipt <PATH>` | Required. |
| `--max-cycles <N>` | Default `4000000000`. |
| `--max-retirements <N>` | Default `2000000000`. |
| `--max-fuel <N>` | Default `4000000000`. |
| `--wall-seconds <N>` | Default `3600`. |
| `--console-bytes <N>` | Default `65536`. |

Plus the [global flags](#global-options).

## Environment Variables

- `RUST_LOG`: Controls logging level (e.g., `info`, `debug`).
  - Native logging uses `tracing`.
  - Example: `RUST_LOG=labwired_core=debug`
- `LABWIRED_API_KEY`: Sent by `test` unless `--no-key` is given.
- `LABWIRED_ESP32S3_FLASH`, `LABWIRED_ESP32S3_ROM`, `LABWIRED_ESP32S3_ROM_DROM`, `LABWIRED_ESP32S3_ROM_ELF`: ROM-boot inputs for `run --rom-boot` and `test`.
- `LABWIRED_ESP32C3_FLASH`: Merged flash image for `test --rom-boot` / `--capture-app-entry` / `--resume-snapshot`.
