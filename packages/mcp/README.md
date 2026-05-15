# @labwired/mcp

> Model Context Protocol server exposing the LabWired deterministic firmware simulator as tools your AI coding agent can call.

LabWired runs ARM Cortex-M / RISC-V / Xtensa firmware against a cycle-accurate, deterministic simulator. This MCP server lets your agent (Claude Code, Cursor, Continue, etc.) **verify firmware behaviour without lying to itself** — same inputs always produce the same outputs, so the agent can iterate against a real oracle.

## What it gives your agent

| Tool | What it does |
|---|---|
| `labwired_catalog` | List supported chips/boards (filter by name). |
| `labwired_simulate` | Run firmware against a test script. Returns `result.json` + UART log. Deterministic. |
| `labwired_validate_system` | Validate a System Manifest YAML before simulating. Fast schema check. |

## Prerequisites

You need the `labwired` CLI on your `PATH`. Install via:

```bash
curl -fsSL https://labwired.com/install.sh | sh
```

Verify with `labwired --help`.

## Install in Claude Code

```bash
claude mcp add labwired -- npx -y @labwired/mcp
```

That's it. Restart Claude Code if the tools don't appear immediately.

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

## Why deterministic matters for agents

Agents that propose firmware fixes hit a fundamental problem: there's no fast, reliable oracle to verify whether a fix actually works. Physical HIL benches are slow and flaky. Best-effort simulators give different results on different runs. LLMs hallucinate "yes I think that fixes it."

LabWired is bit-accurate and reproducible by design: the same firmware + same system config + same test script produce the same `result.json` every time. The agent can propose, verify, iterate, and **trust the answer**.

## Status

Early MVP (v0.1). Three tools, stdio transport, no auth required for local use (the simulator runs on your machine). Worker-side metering for hosted runs lands in a future version.

Issues / requests: <https://github.com/w1ne/labwired/issues>.

## License

MIT.
