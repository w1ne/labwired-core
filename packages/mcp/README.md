# @labwired/mcp

> Model Context Protocol server exposing the LabWired deterministic firmware simulator as tools your AI coding agent can call.

## One-line install (Claude Code)

```bash
claude mcp add labwired -- npx -y @labwired/mcp
```

That's it. Restart Claude Code if the tools don't appear, then ask: *"List LabWired boards and run my ELF on `stm32f103-blinky`."*

Cursor / other MCP clients: see [Other clients](#install-in-cursor) below.

---

LabWired runs ARM Cortex-M / RISC-V / Xtensa firmware against a cycle-accurate, deterministic simulator. This MCP server lets your agent (Claude Code, Cursor, Continue, etc.) **verify firmware behaviour without lying to itself** — same inputs always produce the same outputs, so the agent can iterate against a real oracle.

## What it gives your agent

**Core workflow:**

| Tool | What it does |
|---|---|
| `labwired_list_boards` | High-level: list pre-wired boards (chip + peripherals + demo firmware). Start here. |
| `labwired_run_lab` | High-level: run an ELF on a board by id. Synthesizes the test script, returns cycles + serial + a `snapshot_id`. |
| `labwired_inspect_run` | Fetch detail from a prior `snapshot_id` — `summary`, `serial`, `gpio`, `raw`. 10-min TTL. |
| `labwired_catalog` | Low-level: list raw chip descriptors. |
| `labwired_simulate` | Low-level: run firmware against a custom System Manifest + test script YAML. Full control. |
| `labwired_validate_system` | Validate a System Manifest YAML before simulating. Fast schema check. |
| `labwired_validate_diagram` | Structurally validate a wired diagram (parts + wires) before running. Returns machine-readable diagnostics (`PIN_NOT_ON_CHIP`, `PIN_LACKS_I2C`, `BOARDIO_NOT_TO_MCU`, `NO_MCU`, `COMPONENT_DANGLING`, …) with suggested fixes. |

**Live agent → browser bridge** (optional — for showing humans what the agent is doing):

| Tool | What it does |
|---|---|
| `labwired_create_session` | Open a watch session; returns a `watch_url` like `https://foundry.labwired.com/?watch=<id>`. Subsequent runs mirror to it automatically over WebSocket. |
| `labwired_set_diagram` | Push the current circuit (parts + wires) to the watch session. |
| `labwired_set_source` | Push the current source code to the watch session. |
| `labwired_end_session` | Close the watch session. |

## Prerequisites

You need the `labwired` CLI on your `PATH`. Install via:

```bash
curl -fsSL https://labwired.com/install.sh | sh
```

Verify with `labwired --help`.

## Install in Cursor

Add to `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "labwired": {
      "command": "npx",
      "args": ["-y", "@labwired/mcp"]
    }
  }
}
```

## Install globally (any MCP client)

```bash
npm install -g @labwired/mcp
labwired-mcp   # speaks MCP over stdio
```

## Example agent prompts

Once installed, ask your agent things like:

> *"My STM32F1 blinky firmware hangs after about 100k cycles. Here's the ELF (base64-encoded) and my `system.yaml`. Use LabWired to simulate it with a 200k cycle budget and tell me where it stops."*

> *"Generate a System Manifest for the Nucleo-F401RE with UART2 routed to PA2/PA3 at 115200 baud, then validate it with labwired_validate_system before I commit."*

> *"Here are two versions of my ADC interrupt handler. Run each through labwired_simulate against my regression script and tell me which one passes all assertions."*

## Environment variables

- `LABWIRED_CLI` — override the CLI binary path. Defaults to `labwired` on `PATH`.
- `LABWIRED_REPO_ROOT` — absolute path to your labwired checkout. The high-level `labwired_run_lab` tool resolves board YAMLs from `core/configs/`. Auto-detected when the MCP server is run from inside the repo; required when running globally.

## Why deterministic matters for agents

Agents that propose firmware fixes hit a fundamental problem: there's no fast, reliable oracle to verify whether a fix actually works. Physical HIL benches are slow and flaky. Best-effort simulators give different results on different runs. LLMs hallucinate "yes I think that fixes it."

LabWired is bit-accurate and reproducible by design: the same firmware + same system config + same test script produce the same `result.json` every time. The agent can propose, verify, iterate, and **trust the answer**.

## Status

v0.4 — eleven tools, stdio transport, no auth required for local use (the simulator runs on your machine).

Shipped:

- **v0.1** — `labwired_catalog` / `labwired_simulate` / `labwired_validate_system` (raw chip + YAML workflow).
- **v0.2** — high-level board-centric workflow: `labwired_list_boards` / `labwired_run_lab` / `labwired_inspect_run`. ELF-in only — agents compile locally.
- **v0.3** — live agent → browser bridge: `labwired_create_session` / `labwired_set_diagram` / `labwired_set_source` / `labwired_end_session`. Runs stream over WebSocket to `https://foundry.labwired.com/?watch=<id>`.
- **v0.4** — `labwired_validate_diagram` with machine-readable diagnostics (pin/wire/bus errors + suggested fixes).

Next:

- **v0.5** — Worker-side metering for hosted runs.

Issues / requests: <https://github.com/w1ne/labwired/issues>.

## License

MIT.
