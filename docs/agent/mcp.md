# Connect an agent (MCP)

Connect Claude Code, Codex, Cursor, or another MCP client so an agent can **list boards, run firmware on the twin, and verify** results.

The agent proposes changes. **A green verify or test is the proof** — not a free-form claim that it “looks good.”

---

## Two ways to connect

| Surface | How | Compile in the cloud | Best for |
|---------|-----|----------------------|----------|
| **Hosted** | `https://api.labwired.com/mcp` | Yes (`labwired_compile`) | Cloud agents, no local sim |
| **Stdio (local)** | `npx -y @labwired/mcp` | No — use local build artifacts | Offline / local CLI |

Tool names share the `labwired_*` family. Stdio does **not** expose `labwired_compile`: build locally (or on hosted), then pass a firmware ref or path.

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

Sign in when the client opens a browser — same account as [app.labwired.com](https://app.labwired.com).

### Other HTTP clients

```text
https://api.labwired.com/mcp
```

Use the client’s remote MCP / HTTP flow and finish auth.

Example JSON shape:

```json
{
  "mcpServers": {
    "labwired": {
      "type": "http",
      "url": "https://api.labwired.com/mcp"
    }
  }
}
```

---

## Stdio (local)

### Prerequisites

1. **Node ≥ 20**
2. **`labwired` CLI on `PATH`** for local runs:

```bash
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.22.0 sh
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

### Global binary (optional)

```bash
npm install -g @labwired/mcp
labwired-mcp
```

---

## Smoke check

Ask the agent:

> List LabWired boards with `labwired_list`, then describe `esp32-c3-supermini`.

You should see real catalog entries and pins — not a guessed pinout.

---

## Next

| | |
|--|--|
| [First agent run](first-run.md) | Full list → run → verify loop |
| [Tool reference](tools.md) | What each tool does |
| [Playground](https://app.labwired.com) | Human visual lab |
| [Run firmware (CLI)](../getting_started_firmware.md) | Non-agent path |
| [Fidelity](../fidelity.md) | What a green pass means |
