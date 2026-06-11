# LabWired MCP quality design

Date: 2026-06-11
Status: Approved for implementation
Scope: Improve both hosted and local LabWired MCP surfaces to match the agent-facing quality bar proven by kernelCAD.

## Goal

Make LabWired's MCP connector feel like a first-class agent hardware lab: discoverable, well annotated, OAuth-friendly, and guided enough that an agent can build virtual hardware, validate it, run firmware, inspect evidence, and iterate without guessing.

## Current state

LabWired has two MCP surfaces:

- Hosted MCP in `packages/api/src/mcp`, exposed at `POST /mcp`, authenticated through Clerk OAuth or `lwk_live_` workspace API keys, and backed by the hosted builder for runs.
- Local stdio MCP in `packages/mcp`, published as `@labwired/mcp`, which shells out to a local `labwired` CLI and requires no authentication.

The tool behavior is useful today, but the agent-facing surface is thinner than kernelCAD's. Tool lists lack top-level titles, MCP annotations, a searchable long-tail catalog, and an authoring resource that tells an agent how to use the system. Hosted OAuth discovery exists, but the quality gate should make failures obvious and protect the end-to-end connector flow.

## KernelCAD patterns to copy

1. Tool metadata: every tool has `title`, `annotations.title`, `readOnlyHint`, `destructiveHint`, and `openWorldHint` where appropriate.
2. Progressive discovery: a `search_tools` style meta-tool ranks the full catalog and returns definitions that are immediately callable.
3. Authoring resource: a stable MCP resource exposes the agent guide instead of relying only on README prose.
4. Hosted auth clarity: 401 responses include an OAuth protected-resource metadata pointer, and metadata is tested.
5. Tool behavior remains stable: metadata and discovery improve the connector without rewriting the underlying engine.

## Design

### Shared tool metadata

Create a small metadata helper for each MCP surface:

- Hosted: `packages/api/src/mcp/tool-metadata.ts`
- Local: `packages/mcp/src/tool-metadata.ts`

The helpers provide:

- `toolTitle(name: string): string`
- `toolAnnotations(name: string): { title, readOnlyHint, destructiveHint, openWorldHint? }`
- `decorateTools(tools)` to add top-level `title` and `annotations`

Read-only tools:

- `labwired_search_tools`
- `labwired_list_boards`
- `labwired_list_components`
- `labwired_validate_diagram`
- `labwired_catalog`
- `labwired_validate_system`
- `labwired_inspect_run`

Stateful or external-world tools:

- Hosted: `labwired_start_playground_lab`, `labwired_run`
- Local: `labwired_simulate`, `labwired_run_lab`, `labwired_fuzz`, `labwired_create_session`, `labwired_end_session`, `labwired_set_diagram`, `labwired_set_source`

No current tool is destructive, so `destructiveHint` is always false.

### Search tool

Add `labwired_search_tools` to both surfaces. It uses a dependency-free BM25-lite ranker copied from kernelCAD's `searchTools.ts` pattern, adjusted for LabWired naming. The input schema is:

```json
{
  "type": "object",
  "required": ["query"],
  "properties": {
    "query": { "type": "string" },
    "limit": { "type": "integer", "minimum": 1, "maximum": 25 }
  }
}
```

The output is:

```json
{
  "query": "diagram validation",
  "tools": [
    {
      "name": "labwired_validate_diagram",
      "title": "Validate Diagram",
      "description": "...",
      "inputSchema": { "type": "object", "properties": {} }
    }
  ]
}
```

For this iteration, `tools/list` continues advertising the full catalog plus `labwired_search_tools`. We do not introduce a reduced advertised set yet; LabWired's hosted catalog is small enough that the low-risk win is searchability and consistent metadata.

### Agent guide resource

Add a Markdown guide at `packages/api/src/mcp/resources/labwired-agent-hardware-loop.md` and `packages/mcp/src/resources/labwired-agent-hardware-loop.md`.

Expose it through a stable URI:

```text
labwired://guides/agent-hardware-loop
```

The guide covers:

- Choose a board with `labwired_list_boards`.
- Discover modeled components with `labwired_list_components` where available.
- Build or update the diagram.
- Validate wiring with `labwired_validate_diagram`.
- Compile firmware outside hosted MCP, using documented scaffolds for hosted runs.
- Run with `labwired_run` or `labwired_run_lab`.
- Inspect serial/GPIO/snapshot evidence.
- Iterate until the simulator evidence matches the goal.

Hosted `initialize` advertises `{ tools: {}, resources: {} }`. Hosted HTTP handles `resources/list` and `resources/read`. Local stdio does the equivalent through the MCP SDK request handlers.

### Hosted OAuth quality

Keep the existing authentication model:

- Clerk/OAuth bearer tokens for users.
- `lwk_live_` workspace API keys for CI and non-browser agents.
- 401 responses with `WWW-Authenticate: Bearer realm="LabWired MCP", resource_metadata="..."`.

Improve the quality gate:

- Protected-resource metadata tests must verify `resource`, `resource_name`, `bearer_methods_supported`, `resource_documentation`, CORS headers, and `authorization_servers` when configured.
- A prod-like test should fail if `MCP_AUTHORIZATION_SERVER` is missing in a production environment. This catches the known dead-end where clients discover the protected resource but have no authorization server.
- `initialize` tests must verify resources are advertised.
- An authenticated `tools/list` test must verify every hosted tool has title and annotations.

Implementation should not invent an OAuth provider. Clerk remains the authorization server; this pass makes discovery and connector metadata reliable.

## Error handling

- Unknown search calls return an MCP tool error with JSON text, matching existing hosted/local response style.
- `resources/read` returns a JSON-RPC method error for unknown URIs on hosted HTTP.
- Local resource reads should return an MCP error for unknown URIs through the SDK.
- If guide text cannot be loaded, fail loudly in tests; the guide is shipped source, not optional runtime state.

## Testing

Hosted API tests:

- `mcp-tools.test.ts`: `labwired_search_tools` is advertised, returns relevant tools, and all listed tools include title/annotations.
- `routes.test.ts`: initialize advertises resources; `resources/list` and `resources/read` return the guide; production metadata requires `MCP_AUTHORIZATION_SERVER`.
- `mcp-auth.test.ts`: existing Clerk and API-key paths remain passing.

Local MCP tests:

- `packages/mcp/src/cli.test.ts`: stdio initialize/tools/list includes `labwired_search_tools` and annotated tools.
- Add focused unit tests for the local search ranker/resource handlers if stdio round-trip coverage becomes too coarse.

Full verification:

- `cd packages/api && npm test -- --run`
- `cd packages/mcp && npm test -- --run`
- `cd packages/api && npm run build`
- `cd packages/mcp && npm run build`

## Non-goals

- Do not add hosted compile.
- Do not change simulator behavior, firmware scaffold semantics, quota metering, or builder protocol.
- Do not reduce `tools/list` to a core subset in this pass.
- Do not replace Clerk OAuth.
