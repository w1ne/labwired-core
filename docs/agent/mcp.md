# Connect an agent (MCP)

LabWired exposes a **Model Context Protocol** server so coding agents can **describe hardware, run firmware on the digital twin, and verify** — without grading their own homework.

The agent **proposes**. The **oracle disposes**.

!!! tip "Product vs engine"
    This page is for **using** LabWired from Claude Code, Codex, Cursor, and friends.  
    For contributing to the simulator engine itself, see [Core agents manual](../agents.md).

---

## Two surfaces

| Surface | Endpoint / install | Compile from source | Best for |
|---------|-------------------|---------------------|----------|
| **Hosted** | `https://api.labwired.com/mcp` | Yes (`labwired_compile`) | Cloud agents, zero local sim install |
| **Stdio (local)** | `npx -y @labwired/mcp` | No — flash **artifacts** only | Offline / on-box CLI + `labwired` binary |

Tool names are the same family (`labwired_<verb>`). Stdio **does not** advertise `labwired_compile` — build locally or use hosted, then pass `firmware_ref` / ELF paths. Registry SoT: monorepo `@labwired/board-config` (`mcp-tools.ts`).

---

## Hosted (recommended)

### Claude Code

```bash
claude mcp add labwired --transport http https://api.labwired.com/mcp
```

### Codex

```bash
codex mcp add labwired --url https://api.labwired.com/mcp
```

Sign in / OAuth when the client prompts — same account as [app.labwired.com](https://app.labwired.com).

### Other HTTP MCP clients

Point the client at:

```text
https://api.labwired.com/mcp
```

Use the client’s “remote MCP / HTTP / SSE” flow and complete auth.

---

## Stdio (local)

### Prerequisites

1. **Node ≥ 20**
2. **`labwired` CLI on `PATH`** (local runs):

```bash
curl -fsSL https://labwired.com/install.sh | sh
labwired --help
```

### Claude Code

```bash
claude mcp add labwired -- npx -y @labwired/mcp
```

### Cursor

`~/.cursor/mcp.json` (or project `.cursor/mcp.json`):

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

### Global binary

```bash
npm install -g @labwired/mcp
labwired-mcp
```

---

## Smoke check

Ask the agent:

> List LabWired boards with `labwired_list`, then describe `esp32-c3-supermini` (or `esp32c3`).

You should see catalog entries and pin/toolchain metadata — not a hallucinated pinout.

Next: [First agent run](first-run.md) · [Tool reference](tools.md)

---

## Related

| | |
|--|--|
| [Playground](https://app.labwired.com) | Human visual lab |
| [CLI — running firmware](../getting_started_firmware.md) | Non-agent path |
| [CI integration](../ci_integration.md) | Oracle in pipelines |
| [Fidelity](../fidelity.md) | What a green pass means |
