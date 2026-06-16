# LabWired Agent Runtime Contract

This contract defines how MCP clients, Playground links, and firmware execution fit together.

## Diagram Contract

Agents exchange hardware as `LabWiredDiagramV1`.

The canonical schema and normalizer live in `@labwired/board-config`:

- `LABWIRED_DIAGRAM_V1_SCHEMA`
- `normalizeLabWiredDiagramV1(input)`

Every diagram returned in a Playground URL must be normalized before it leaves the MCP API. The normalized shape always includes:

- `version: 1`
- `board`
- `parts[].id`
- `parts[].type`
- `parts[].x`
- `parts[].y`
- `parts[].rotate`
- `parts[].attrs`
- `wires[].from`
- `wires[].to`
- `wires[].color`

Compact agent input is accepted as compatibility sugar, but it is not a valid outbound share payload until normalized.

## Viewing Contract

MCP tools return two viewer paths:

- `inline_frame_url`: an embeddable Playground URL, currently `https://app.labwired.com/?embed=true#...`
- `studio_url`: the full Playground URL, currently `https://app.labwired.com/#...`

The ChatGPT MCP component must embed the real Playground iframe. It must not maintain a separate fake renderer for board components.

## Execution Contract

Firmware execution does not happen in the browser or inside ChatGPT.

The runtime path is:

1. MCP client calls `labwired_run`.
2. `api.labwired.com` validates request shape, compiles the diagram to simulator manifests, meters usage, and dispatches execution.
3. `builder.labwired.com` runs the firmware ELF against the compiled virtual hardware.
4. The Worker returns serial output, cycle counts, exit status, and diagnosis.

The Cloudflare Worker is the orchestrator. The builder service is the execution runtime.

## Deployment Contract

Playground production builds require `VITE_CLERK_PUBLISHABLE_KEY` unless `VITE_DISABLE_AUTH=true` is explicitly set for local development. The build guard is `packages/playground/scripts/verify-production-env.mjs`.
