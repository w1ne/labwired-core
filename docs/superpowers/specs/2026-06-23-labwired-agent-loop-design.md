# LabWired Agent-Loop Closure — Design

Date: 2026-06-23
Status: Approved (brainstorming) — pending spec review
Branch: `feat/agent-loop-close` (worktree `~/projects/labwired-agent-loop`)

## Problem

LabWired's wedge is the agent-native embedded lab: an LLM should run the full
loop — write firmware → compile → run on the hardware oracle → observe → diagnose
→ fix — without ever hitting a wall it cannot get past on its own. Today the loop
dead-ends. This was confirmed empirically by driving the live hosted MCP
(`https://mcp.proto.cat/mcp`) as a first-time agent.

### Evidence (dogfooding the live MCP, 2026-06-23)

- `protocat_plan_device` returns clean ordered phases. Works.
- `protocat_compile_firmware` compiled ESP32 Arduino source to a real ELF
  (`ok:true`, `elf_base64`). Works.
- `labwired_run_build` with a *reasonable guessed* manifest
  (`chip: esp32`, `peripherals: [{type: led, pin: 2}]`) returned
  `status: error`, `stopReason: config_error`, with the diagnosis:
  *"Simulation failed at configuration time — the system YAML or chip descriptor
  could not be loaded. Check the diagram and target configuration."*
  The real schema is `name` / `chip` / `external_devices` /
  `board_io[{peripheral, pin, kind, signal, active_high}]` — unguessable, and
  the error names neither the bad field nor the schema nor a fix.
- `labwired_run_example` (`esp32c3-mlx90640-thermal`) dropped the connection at
  **1:00.41** ("Remote end closed connection without response"); a second
  attempt hung past 2 minutes. A real oracle run exceeds a ~60s window and the
  agent gets **no verdict at all**.

### The five confirmed dead-ends

1. The system manifest is unguessable and undocumented — no tool returns its
   schema.
2. The manifest `chip:` field wants a filesystem path
   (`"../../configs/chips/stm32f103.yaml"`) — impossible for a remote agent with
   no filesystem and no exposed chip-by-id catalog.
3. The run-path failure diagnosis (`config_error`) is non-actionable.
4. Long runs return a dropped socket instead of a verdict (~60s ceiling).
5. ESP32 is half-supported: the planner accepts it and compile works, but every
   curated example is STM32 and nothing tells the agent what the oracle can run.

## Key finding: most of the cure already exists, just unexposed

The labwired codebase already contains the building blocks; they are not wired to
the agent surface:

- `labwired_list_boards`, `labwired_catalog` / `listChips`, `labwired_validate_system`
  exist (`packages/mcp/src/index.ts`, `packages/mcp/src/cli.ts`,
  `packages/api/src/mcp/tools.ts`) but are NOT among the 6 tools exposed on
  `mcp.proto.cat`.
- A structured diagnostic type already exists and is used for the diagram path:
  `Diagnostic { code, severity, message, hint, subjects }`
  (`packages/board-config/src/erc/diagnostic.ts`), produced by
  `composeDiagnostics()`. The run path ignores it.
- The `config_error` text is a hardcoded generic string at
  `services/labwired-builder/src/run.ts:118` inside `buildDiagnosis()`.
- The manifest schema is the Rust `SystemManifest` struct
  (`core/crates/config/src/lib.rs:167-187`), with `ChipDescriptor` (96-115),
  `ExternalDevice` (118-124), `BoardIoBinding` (151-164).
- Chip catalog: `core/configs/chips/*.yaml`, resolved via
  `packages/mcp/src/boards.ts` (`BOARDS` 35-109, `getBoard` 128-130).
- Run path: gateway → `builderRun()` (`packages/api/src/mcp/builder-client.ts:33`)
  → builder `/run` (`services/labwired-builder/src/server.ts:70`) →
  `run()` (`services/labwired-builder/src/run.ts:154`), sim subprocess
  `timeout: 60000` at `run.ts:207`. No async/job/polling mechanism exists.

## Repos in scope

- **`labwired`** (this worktree): builder diagnostics, manifest schema source,
  chip-by-id resolution, async job model, support-matrix data.
- **`protocat-private`** (the umbrella gateway, `mcp-http-server.ts` on Europa):
  the 6-tool agent surface (`protocat_*`, `labwired_run_build`,
  `labwired_run_example`). New `labwired_lookup` and `labwired_run_status` tools,
  and the schema/chip-id wiring, are exposed here.

The `protocat_*` tools are NOT in the labwired repo; they federate into the
labwired builder + API from the gateway repo.

## Design

### Part A — Authoring (kill the blind-guess wall)

**A1. `labwired_lookup` tool** (gateway). One read-only tool the agent calls
before authoring, selected by `of`:
- `manifest_schema` → the `SystemManifest` schema (from
  `core/crates/config/src/lib.rs:167-187`) plus a complete worked example, and an
  explicit statement that `chip` is an **id**, not a path.
- `chips` → proxies `listChips` / `labwired_list_boards`, returning real chip ids,
  their peripheral names (`gpioa`, `i2c1`, …), and valid `kind`/`signal` vocab.
- `examples` → lists curated `example_id`s so `run_example` is not a guess.

Mirrors the kernelCAD `lookup_api` / `lookup_cookbook` / `lookup_diagnostics`
pattern already live on the same gateway.

**A2. Path-free chip reference** (gateway + builder). `run_build` already accepts
`chip_id` separately and builder `/run` accepts `chipYaml` separately, so the
plumbing exists. Make the manifest `chip:` field accept a bare id the gateway
resolves to a descriptor; update `plan_device` output and the schema example to
show the id form. Eliminate the filesystem-path requirement from the agent's view.

**A3. Actionable `config_error`** (builder). Replace the hardcoded string at
`services/labwired-builder/src/run.ts:118` with the structured `Diagnostic` shape
already used in `board-config`: name the offending field, the expected schema
fragment, and a fix hint (e.g. *"`board_io[0].peripheral: "gpio2"` is not a
peripheral on `esp32c3`; valid: gpio0…gpio21"*), and point the agent at
`labwired_lookup`.

### Part B — Reliability (a verdict every time)

**B1. Async long-run** (gateway + builder). Add a job model so runs exceeding the
proxy idle window still return: `run_build` / `run_example` return a `job_id`
immediately when a run will be long; the agent polls a new `labwired_run_status`
tool until `done`, then receives the full verdict (serial, peripherals, diagnosis).

**Open risk to resolve as the FIRST implementation task:** confirm the exact
source of the ~60s drop. Hypothesis: a front-proxy idle timeout on the Europa
gateway (Cloudflare/Caddy), not the sim subprocess — because a sim killed at
`run.ts:207` should still return a `timedOut` result, not drop the socket. If it
is the proxy, the async job model is mandatory. If somehow it is the sim, a
timeout bump may suffice. Do not build B blind; measure first.

### Part C — Honest support matrix (folds in as docs)

`labwired_lookup of:chips` marks ESP32 vs STM32 capability explicitly (compile-only
vs full oracle run), so the planner never sells a path the oracle cannot run.

## Testing

- Regression from the dogfood script: clueless-agent guessed manifest → must now
  return an actionable error that names the offending field and the fix; a
  corrected manifest → must close the loop and return a verdict.
- `labwired_lookup of:manifest_schema` → returns schema + a worked example that,
  used verbatim with a real chip id, runs clean.
- Long-run (`run_example` thermal) → returns a verdict via the job path, not a
  dropped socket.
- ESP32 path: `labwired_lookup of:chips` reports ESP32 capability honestly;
  planner output matches what the oracle can run.

## Out of scope (explicitly not this wedge)

RTL co-simulation, cycle accuracy, hardware-emulator backends, pre-silicon SoC
assembly, SystemC/TLM ingestion. These are the Cadence/VLAB product category and
target chip vendors, not LabWired's agent-native audience. Not part of "perfect"
for this wedge.

## Sequencing

A first (most agents die here, earliest in the loop; near-direct port of patterns
already live on the gateway), then B (verdict reliability), then C (docs, folds
into A1's `chips` output).
