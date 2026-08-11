# MCP tool reference

Tool names are `labwired_<verb>`. **Hosted** (`https://api.labwired.com/mcp`) and **stdio** (`@labwired/mcp`) share the same family; not every tool appears on both surfaces.

If this page and the live server disagree, **the live server wins** — fix the docs.

---

## Core loop (use these first)

| Tool | Hosted | Stdio | Role |
|------|:------:|:-----:|------|
| `labwired_search` | ✅ | ✅ | Search tools by keyword |
| `labwired_list` | ✅ | ✅ | List boards / MCUs / components |
| `labwired_describe` | ✅ | ✅ | Pins, buses, attrs for a catalog id |
| `labwired_validate` | ✅ | ✅ | Diagram / wiring checks before run |
| `labwired_compile` | ✅ | ❌ | Source → `firmware_ref` (+ ESP flash images) |
| `labwired_run` | ✅ | ✅ | Run firmware; serial, GPIO, snapshot |
| `labwired_inspect` | ✅ | ✅ | Deeper state from a `snapshot_id` |
| `labwired_verify` | ✅ | ✅ | **Pass/fail oracle** |
| `labwired_lab` | ✅ | ❌ | Studio lab widget + share URL |

**Rule:** do not claim success until `labwired_verify` (or an explicit test assertion) is green.

---

## Discovery and parts

| Tool | Hosted | Stdio | Role |
|------|:------:|:-----:|------|
| `labwired_catalog` | ✅ | — | Catalog snapshot |
| `labwired_part` | ✅ | ✅* | Lookup by MPN / alias / id |
| `labwired_datasheet` | ✅ | ✅* | Datasheet text (paged) |
| `labwired_export` | ✅ | ✅ | KiCad netlist / schematic / BOM / … |
| `labwired_validate_device` | ✅* | ✅* | Validate a device YAML before merge |

\*May vary by surface; if missing on stdio, use hosted or the CLI.

---

## Typical sequences

**Blink / UART demo**

```text
list → describe board → compile (hosted) → run → verify (serial contains …)
```

**Sensor / actuator**

```text
list components → describe part → validate diagram → run (+ stimuli if needed) → verify
```

**New part authoring**

```text
validate_device (YAML) → attach on a known board → run → verify → document
```

See [Onboard a part](../howto/onboard-part.md).

---

## Deprecated names

Do not use old aliases if the server still accepts them for compatibility. Prefer the `labwired_*` names in the tables above. If a client shows only legacy names, update the MCP package or connection URL.

---

## Next

- [Connect MCP](mcp.md)
- [First agent run](first-run.md)
- [Fidelity](../fidelity.md)
