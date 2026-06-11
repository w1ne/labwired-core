# LabWired MCP Quality Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add kernelCAD-quality MCP metadata, tool search, guide resources, and OAuth discovery gates to LabWired hosted and local MCP.

**Architecture:** Keep hosted and local MCP behavior intact, but wrap each catalog with shared metadata/search/resource helpers. Hosted HTTP gains resources/list/read and stricter OAuth metadata behavior; local stdio gains matching tool metadata/search/resource support through the MCP SDK.

**Tech Stack:** TypeScript, MCP SDK, Cloudflare Worker-style hosted API tests, Vitest, Node stdio MCP tests.

---

## File Structure

- Modify `packages/api/src/mcp/tools.ts`: add search dispatch and decorated hosted tool listing.
- Create `packages/api/src/mcp/search-tools.ts`: BM25-lite tool ranker copied from kernelCAD's pattern.
- Create `packages/api/src/mcp/tool-metadata.ts`: hosted tool titles and annotations.
- Create `packages/api/src/mcp/resources.ts`: hosted resource descriptors and guide reader.
- Create `packages/api/src/mcp/resources/labwired-agent-hardware-loop.md`: hosted agent guide.
- Modify `packages/api/src/mcp/http.ts`: advertise resources and handle resources/list/read.
- Modify `packages/api/src/mcp/oauth.ts`: make production missing authorization server fail clearly.
- Modify `packages/api/tests/mcp-tools.test.ts`: hosted search and annotations tests.
- Modify `packages/api/tests/routes.test.ts`: OAuth metadata/resource tests.
- Modify `packages/mcp/src/index.ts`: local search/resource handlers and decorated tool listing.
- Create `packages/mcp/src/search-tools.ts`: local BM25-lite ranker.
- Create `packages/mcp/src/tool-metadata.ts`: local tool titles and annotations.
- Create `packages/mcp/src/resources.ts`: local resource descriptors and guide reader.
- Create `packages/mcp/src/resources/labwired-agent-hardware-loop.md`: local agent guide.
- Modify `packages/mcp/src/cli.test.ts`: local stdio metadata/search/resource smoke tests.

## Tasks

### Task 1: Hosted MCP Search and Metadata

**Files:**
- Create: `packages/api/src/mcp/search-tools.ts`
- Create: `packages/api/src/mcp/tool-metadata.ts`
- Modify: `packages/api/src/mcp/tools.ts`
- Test: `packages/api/tests/mcp-tools.test.ts`

- [ ] **Step 1: Write failing hosted metadata/search tests**

Add tests that assert `labwired_search_tools` is listed, every listed tool has `title` and `annotations`, and a query for "diagram validation" returns `labwired_validate_diagram`.

- [ ] **Step 2: Run failing hosted MCP tools tests**

Run: `cd packages/api && npm test -- --run tests/mcp-tools.test.ts`
Expected: FAIL because `labwired_search_tools` and annotations do not exist.

- [ ] **Step 3: Implement hosted search and metadata**

Copy kernelCAD's BM25-lite ranker shape into `search-tools.ts`, add `tool-metadata.ts`, decorate `listHostedTools()`, and handle `labwired_search_tools` in `dispatchHostedTool()`.

- [ ] **Step 4: Run hosted MCP tools tests**

Run: `cd packages/api && npm test -- --run tests/mcp-tools.test.ts`
Expected: PASS.

### Task 2: Hosted MCP Resources and OAuth Quality

**Files:**
- Create: `packages/api/src/mcp/resources.ts`
- Create: `packages/api/src/mcp/resources/labwired-agent-hardware-loop.md`
- Modify: `packages/api/src/mcp/http.ts`
- Modify: `packages/api/src/mcp/oauth.ts`
- Test: `packages/api/tests/routes.test.ts`

- [ ] **Step 1: Write failing hosted resource/OAuth tests**

Add tests that initialize advertises resources, `resources/list` returns `labwired://guides/agent-hardware-loop`, `resources/read` returns Markdown containing "LabWired agent hardware loop", and production metadata without `MCP_AUTHORIZATION_SERVER` returns a 500 diagnostic instead of silently omitting `authorization_servers`.

- [ ] **Step 2: Run failing route tests**

Run: `cd packages/api && npm test -- --run tests/routes.test.ts`
Expected: FAIL because resources are not advertised/handled and missing prod auth server is not an error.

- [ ] **Step 3: Implement hosted resources and stricter OAuth metadata**

Add hosted resource registry, advertise `{ resources: {} }` in initialize, handle `resources/list` and `resources/read`, and make `handleMcpProtectedResourceMetadata()` return 500 in production-like environments when `MCP_AUTHORIZATION_SERVER` is unset.

- [ ] **Step 4: Run hosted route tests**

Run: `cd packages/api && npm test -- --run tests/routes.test.ts`
Expected: PASS.

### Task 3: Local MCP Search, Metadata, and Resources

**Files:**
- Create: `packages/mcp/src/search-tools.ts`
- Create: `packages/mcp/src/tool-metadata.ts`
- Create: `packages/mcp/src/resources.ts`
- Create: `packages/mcp/src/resources/labwired-agent-hardware-loop.md`
- Modify: `packages/mcp/src/index.ts`
- Test: `packages/mcp/src/cli.test.ts`

- [ ] **Step 1: Write failing local stdio tests**

Expand `cli.test.ts` so initialize/tools/list checks `labwired_search_tools`, tool titles/annotations, and resources/list/read for `labwired://guides/agent-hardware-loop`.

- [ ] **Step 2: Run failing local MCP tests**

Run: `cd packages/mcp && npm test -- --run src/cli.test.ts`
Expected: FAIL because local search/resource metadata is not implemented.

- [ ] **Step 3: Implement local search, metadata, and resources**

Add the same BM25-lite ranker and guide resource to the local package, decorate `tools/list`, add `labwired_search_tools` dispatch, and register `resources/list` and `resources/read` request handlers.

- [ ] **Step 4: Build then run local MCP tests**

Run: `cd packages/mcp && npm run build && npm test -- --run src/cli.test.ts`
Expected: PASS.

### Task 4: Full Verification

**Files:**
- Verify all touched hosted/local files.

- [ ] **Step 1: Run hosted tests**

Run: `cd packages/api && npm test -- --run`
Expected: PASS.

- [ ] **Step 2: Run local tests**

Run: `cd packages/mcp && npm test -- --run`
Expected: PASS.

- [ ] **Step 3: Run hosted typecheck**

Run: `cd packages/api && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 4: Run local build**

Run: `cd packages/mcp && npm run build`
Expected: PASS.

- [ ] **Step 5: Review git diff**

Run: `git status --short && git diff -- packages/api/src/mcp packages/api/tests packages/mcp/src docs/superpowers`
Expected: only scoped MCP/spec/plan changes plus pre-existing unrelated dirty paths in status.

## Self-Review

- Spec coverage: Tasks cover hosted/local metadata, search, resources, and OAuth quality gates.
- Placeholder scan: No task depends on unspecified files or future decisions.
- Type consistency: Hosted/local tool shape uses existing `McpTool` style plus MCP-compatible `title` and `annotations` fields.
