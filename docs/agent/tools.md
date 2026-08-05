# MCP tool reference

Canonical names are `labwired_<verb>`. Both **hosted** (`https://api.labwired.com/mcp`) and **stdio** (`@labwired/mcp`) load tools from the same registry; surface tags decide what each client advertises.

!!! note "Living contract"
    Shapes live in monorepo `packages/board-config/src/mcp-tools.ts`. If this page and the live server disagree, **the server wins** — file a docs fix.

---

## Core loop (use these first)

| Tool | Hosted | Stdio | Role |
|------|:------:|:-----:|------|
| `labwired_search` | ✅ | ✅ | Search the tool catalog by keyword |
| `labwired_list` | ✅ | ✅ | List boards / MCUs / components (`kind` optional) |
| `labwired_describe` | ✅ | ✅ | Pins, buses, attrs for any catalog id |
| `labwired_validate` | ✅ | ✅ | ERC / diagram checks before run |
| `labwired_compile` | ✅ | ❌ | Source → `firmware_ref` (+ ESP flash images) |
| `labwired_run` | ✅ | ✅ | Execute firmware on the twin; observe serial/GPIO/… |
| `labwired_inspect` | ✅ | ✅ | Deeper state from a prior `snapshot_id` |
| `labwired_verify` | ✅ | ✅ | **Oracle** — pass/fail, not model self-report |
| `labwired_lab` | ✅ | ❌ | Open / share a Studio lab widget + URL |

**Rule for agents:** never claim success until `labwired_verify` (or an explicit oracle assertion in the run result) is green.

---

## Discovery & parts

| Tool | Hosted | Stdio | Role |
|------|:------:|:-----:|------|
| `labwired_catalog` | ✅ | — | Versioned Circuit V1 catalog snapshot |
| `labwired_resolve_circuit` | ✅ | — | Resolve CircuitRequestV1 → validated graph |
| `labwired_part` | ✅ | ✅* | Lookup part by MPN / alias / catalog id |
| `labwired_part_citation` | ✅ | ✅* | Evidence behind a knowledge claim |
| `labwired_datasheet` | ✅ | ✅* | Read manufacturer datasheet text (paged) |
| `labwired_export` | ✅ | ✅ | Export diagram (KiCad netlist / schematic / BOM / …) |

\*Advertisement may follow registry `surfaces`; if a tool is missing on stdio, use hosted or the CLI.

---

## Power / advanced

| Tool | Hosted | Stdio | Role |
|------|:------:|:-----:|------|
| `labwired_put_source` | ✅ | — | Store source tree as content-addressed refs |
| `labwired_debug` | ✅ | ✅* | Scripted breakpoints / probe on the twin |
| `labwired_fuzz` | — | ✅ | Coverage-guided fuzz; crashes are replayable |
| `labwired_ingest_svd` | — | ✅ | SVD → declarative peripheral YAML |
| `labwired_validate_device` | — | ✅ | Validate declarative device YAML (no persist) |
| `labwired_project` | ✅ | — | Create / update / publish Studio projects |

---

## Currency: firmware refs

Hosted `labwired_compile` returns a **`firmware_ref`** (`sha256:<hex>`).  
`labwired_run` / `labwired_verify` / fuzz accept that ref (or a lab document’s artifact ref).

Stdio resolves refs from the **local artifact cache**, then the hosted blob API when needed. Prefer refs over stuffing multi‑MB binaries through the model context.

---

## Stdio vs hosted run/verify

| | Hosted | Stdio |
|--|--------|-------|
| Input | Source compile-then-run **or** `firmware_ref` | **Artifact-only** (`firmware_ref` / path + target) |
| Diagram | Supported on run/lab paths | Diagram validate + local system yaml patterns |
| Stimuli | Supported on `labwired_run` | Supported |
| Snapshot | `snapshot_id` for `labwired_inspect` | In-proc snapshot store |

---

## Deprecated names (do not use)

Old names (`labwired_list_boards`, `labwired_run_device`, `labwired_inspect_run`, `search_tools`, …) are **removed**. Hard rename only — no aliases.

---

## Next

- [Connect MCP](mcp.md)  
- [First agent run](first-run.md)  
- Board fidelity: [ESP32-C3](../boards/esp32c3.md)  
