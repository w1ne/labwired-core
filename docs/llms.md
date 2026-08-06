# llms.txt — LabWired for agents

```
# LabWired
> Deterministic firmware digital twin. Agent proposes; oracle disposes.

## Product
- Studio / Playground: https://app.labwired.com
- Docs: https://docs.labwired.com
- Hosted MCP: https://api.labwired.com/mcp
- Stdio MCP: npx -y @labwired/mcp

## Install MCP
- Claude Code (hosted): claude mcp add labwired --transport http https://api.labwired.com/mcp
- Claude Code (stdio): claude mcp add labwired -- npx -y @labwired/mcp
- Codex: codex mcp add labwired --url https://api.labwired.com/mcp

## Core tools (labwired_*)
- labwired_search, labwired_list, labwired_describe
- labwired_validate, labwired_compile (hosted only)
- labwired_run, labwired_inspect, labwired_verify
- labwired_lab (hosted), labwired_fuzz (stdio), labwired_debug

## Rule
Never claim firmware success without labwired_verify (or explicit oracle green).

## Docs map
- Agent: /agent/mcp/ /agent/tools/ /agent/first-run/
- Fidelity: /fidelity/
- Boards: /boards/esp32c3/ /boards/nrf52840/ /boards/rp2040/
- Parts: /parts/
```

Human pages: [Connect MCP](agent/mcp.md) · [Tools](agent/tools.md) · [First run](agent/first-run.md).
