# Hosted MCP build→run loop — design

> **Pivot 2026-05-31: run-only.** Hosted compile dropped (untrusted-code liability); agents compile in their own sandbox (see docs/firmware-scaffolds) and upload the ELF. LabWired hosts the digital-twin run + a hardware-level failure diagnosis. Positioning unchanged: digital twins for HW simulation + agent grounding.

**Date:** 2026-05-31
**Status:** Approved design, ready for implementation planning
**Scope:** Expand the hosted MCP (`packages/api/src/mcp`) so an agent can drive the full firmware loop — write code → compile → build a device → run → read results — headlessly.

## Goal

An agent (Claude Code, ChatGPT, Cursor, the OpenAI/Claude SDKs — anything that speaks MCP and authenticates via the existing Clerk OAuth flow) can, with no human browser in the loop:

1. Write firmware source.
2. Compile it for a real supported target.
3. Build a device (MCU + peripherals + wiring).
4. Run the firmware in the deterministic LabWired engine.
5. Read results — serial output, run status/stop reason, and peripheral proof-of-life (display framebuffer, GPIO/LED state).

## Non-goals (explicitly deferred to phase 2)

- Rust firmware compilation (slow, needs Cargo projects + crate fetching + an airgap-vs-network conflict — see Risks).
- Targets beyond the v1 chip (RISC-V, ESP32/Xtensa, the rest of the STM32/nRF family).
- A one-shot wrapper tool (`run_lab`). Granular tools first; add convenience later only if it earns its place.
- Live browser-watch visualization via the v0.3 session bridge (orthogonal; can layer on later).
- Autoscaling / multi-box build farm. One Hetzner box, with explicit concurrency + quota limits.

## v1 scope — one proven path, end to end

- **Language:** C/C++ only. The existing `packages/playground/server/compile-server.ts` already compiles single-file C with `arm-none-eabi-gcc` in <1s. That is the fast, proven path.
- **Target:** `stm32l476` only. It is the HW-validated chip (J-Link parity, the L476 Breakout demo) with a known-good `core/configs/chips/stm32l476.yaml`.
- Prove the whole loop on this single path. Fanning out to more targets is mechanical once the loop is real; fanning out before it's proven multiplies unproven surface.

## Architecture

```
agent ──JSON-RPC/HTTP──▶ MCP Worker  (packages/api, EXISTING Cloudflare Worker)
                           • Clerk bearer auth (exists)
                           • tool routing (exists; +compile/run/list_components)
                           • diagram → system.yaml (shared pure converter)
                           • per-workspace rate-limit + quota enforcement
                           • holds BUILDER_SECRET; proxies compile/run
                              │  Cloudflare Tunnel  (builder.labwired.com, no public inbound port)
                              ▼
                         labwired-builder  (NEW, small HTTP service on the Hetzner box)
                           • POST /compile  → sandbox → arm-none-eabi-gcc → ELF bytes
                           • POST /run      → native `labwired` CLI → result.json + uart.log + snapshot
                           • shared-secret header check; otherwise a dumb, stateless job runner
                              │
                              ▼
                         toolchain (arm-none-eabi-gcc) + native `labwired` binary, on the box
```

Three components, one new (`labwired-builder`). The Worker never compiles or runs (a V8 isolate can't); it stays a thin authenticated front door. The box does the work and keeps the **native** cycle-accurate engine, which is the product's core value.

### Why Hetzner, not Cloudflare Containers

The box is already paid for, persistent (no cold starts), and trivially holds multi-GB toolchains. CF Containers bill provisioned memory while awake (~$13/mo if kept warm) and choke on large images / cold starts — and offer nothing a non-latency-sensitive job runner needs. Decision recorded; not revisiting in v1.

### Worker ↔ builder link

Cloudflare Tunnel: `cloudflared` runs on the box, dials out to Cloudflare, exposes the builder at `builder.labwired.com` with **no public inbound port**. The Worker calls it with an `X-Builder-Secret` header (a Worker secret the builder verifies). Defence in depth: tunnel + secret.

## Tool surface (granular — 3 new, 2 existing)

All tools require the existing Clerk bearer auth and count against the workspace quota.

### `labwired_compile`
- **In:** `{ source: string, language: "c" | "cpp", target: "stm32l476" }`
- **Out:** `{ ok: boolean, elf_base64?: string, size_bytes?: number, errors: Diagnostic[] }`
- `Diagnostic = { file?, line?, col?, severity: "error"|"warning", message }` parsed from gcc stderr.
- **No server-side ELF handle.** The ELF (10 KB–1 MB) is returned as base64 and the agent passes it back into `run`. The builder stays stateless — no `elf_id`, no TTL, no eviction, no "unknown id" failures across restarts.

### `labwired_run`
- **In:** `{ elf_base64: string, diagram: Diagram, max_steps: number }`
- **Out:** `{ status, stop_reason, stop_reason_details, steps_executed, cycles, instructions, serial: string, peripherals: { id, type, state }[], assertions?, timed_out: boolean }`
- Maps directly onto the CLI's existing `TestResult` (`result.json`) + `uart.log` + peripheral snapshots. `peripherals[].state` carries proof-of-life — e.g. the PCD8544 framebuffer or LED/GPIO levels.
- The Worker converts `diagram` → `system.yaml`, then the builder runs the native CLI in its JSON/artifacts mode and returns the parsed result. `timed_out` is true when `stop_reason` is the step limit (so truncation is never silent).

### `labwired_validate_diagram` (existing, reused)
Wiring/structure diagnostics before a run.

### `labwired_list_boards` (existing, expanded)
Returns `{ board, target, language[] }` triples for what's actually buildable (v1: just the stm32l476 board, C/C++).

### `labwired_list_components` (new, small)
Returns the peripherals the **sim actually models** (from the chip/component registry), so the agent only wires devices that can be simulated. Closes the "build a device the sim can't run" gap that `validate_diagram` doesn't catch.

### Consistency guard
`run` rejects (clear error, no execution) when the compile `target`, the `diagram.board`'s chip, and the generated `system.yaml` chip disagree. No silent mismatches.

## Firmware scaffolding (the real "compile from source" work)

Bare-metal C is not a single free-standing file. v1 ships a **fixed per-target scaffold** for stm32l476: startup code, vector table, and linker script (`memory.x`/`.ld`) baked into the builder. The agent supplies application-level C (`main()` and friends); the builder compiles it against the scaffold with the known-good flags from `compile-server.ts` (`-mthumb -mcpu=cortex-m4` etc.). This is explicit and documented, not magic. Adding a target = adding its scaffold.

## diagram → system.yaml

Today the converter (`diagramToConfig`) lives in `packages/ui` (a React/browser package) and cannot be imported cleanly into the Worker. **Extract the pure conversion logic into a small shared, dependency-free module** consumed by both the UI and the Worker. This is real work, not a free `import` — budgeted as its own task.

## Security & abuse

- **Compile sandbox:** agent source is untrusted. C compilation is comparatively safe (gcc doesn't execute arbitrary code; `#include` is the main risk), but still runs in a locked-down sandbox: ephemeral workdir, **no network** (fine for single-file C — this is *why* Rust is deferred, since its builds need crates.io and execute `build.rs` on the host), CPU/memory/wall limits, bounded include paths.
- **Run is inherently sandboxed:** firmware executes inside the emulator, never on the host.
- **Abuse control:** DCR is enabled, so anyone can OAuth-register and reach these tools — i.e. the builder is internet-reachable free compute on a personal box. The Worker enforces a **per-workspace rate limit** and **counts compiles + runs against the existing workspace quota** (extend the `/v1/runs` metering to cover builds). Concurrency on the box is capped (a fixed worker pool / semaphore); excess requests queue or get a clear "busy" error rather than OOMing the box.
- The builder runs as an unprivileged user; a sandbox escape is contained to that user, not root.

## Long runs

`run` is synchronous and bounded by `max_steps`. A documented default cap keeps calls under typical MCP/agent client timeouts; exceeding it returns `timed_out: true` (never a silent truncation). A job+poll async pattern is **deferred** — revisit only if real firmware boots routinely exceed the cap.

## Error handling

| Failure | Behaviour |
|---|---|
| Compile error | `compile` → `ok:false` + structured `errors[]` |
| Invalid wiring | `run`/`validate_diagram` → reuse existing diagnostics |
| Unmodeled component | caught by `list_components` guidance + a `run` diagnostic |
| target/board/chip mismatch | `run` → consistency-guard error, no execution |
| Step-limit hit | `run` → `timed_out: true`, partial results returned |
| Builder unreachable / bad secret | tool → clear transport error |
| Over quota / rate limit | tool → quota error with usage info |

## Testing

- **Worker (unit):** tool routing, auth, quota/rate-limit, diagram→system.yaml, consistency guard — builder mocked.
- **Shared converter (unit):** diagram→system.yaml golden cases incl. unmodeled components.
- **Builder (contract):** compile a tiny C blink for stm32l476 → ELF; run a known ELF → expected `serial` + `stop_reason`.
- **e2e (gated):** C blink → compile → run on stm32l476 → assert serial + clean stop, mirroring the existing `core/crates/core/tests/e2e_nokia5110_invaders.rs` pattern.
- **Sandbox:** a compile that attempts network / excessive resources is contained (no host network egress; killed at the wall/mem limit).

## Risks / open questions

1. **Exact CLI invocation for `run`.** Verified the CLI emits `result.json` (`TestResult`), `uart.log`, and peripheral `state` snapshots, and accepts `--max-steps`/`--json`/artifacts. The precise flag combination + whether peripheral snapshots need a script is settled during implementation; the *capability* is confirmed.
2. **Scaffold correctness.** The stm32l476 C scaffold (startup/linker/vectors) must boot to `main()` in-sim; validate against an existing known-good example before exposing the tool.
3. **Quota semantics.** Decide whether a failed compile costs quota (proposal: compiles are cheap/free up to a rate cap; only successful runs meter cycles, consistent with `/v1/runs`).
4. **Builder deploy/runbook.** Installing the toolchain + `cloudflared` + the service as a managed unit on the box is ops work to capture in the implementation plan.
