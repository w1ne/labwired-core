# Wiring Kernel C: Compiler + Surfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `compile(diagram) → { systemYaml, chipYaml, diagnostics }` with deviceClass dispatch and net-derived bus binding; `labwired_validate_diagram` upgraded and `labwired_compile_diagram` added on BOTH MCP surfaces with a parity test; the SpiceDispenser hero diagram compiles to a runnable manifest.

**Architecture:** `src/compile/` in board-config wraps and supersedes `diagram-to-config.ts` (which becomes a thin deprecated wrapper). ERC errors abort compile. Local MCP (`packages/mcp`) and hosted API (`packages/api`) call the same kernel functions. IRQ ordinal validation is a compile-time data-driven check.

**Tech Stack:** TypeScript ~5.9, vitest; packages/mcp + packages/api tooling as found.

**Spec:** `docs/superpowers/specs/2026-06-12-wiring-kernel-slice2-design.md` (Section 5 + Error handling + Testing)

**CRITICAL — workspace:** ALL work in `/home/andrii/projects/labwired/.worktrees/feat-wiring-kernel-slice2` (branch `feat/wiring-kernel-slice2`). Another agent works in the main checkout — NEVER touch `/home/andrii/projects/labwired` or its `core/`. The worktree's own `core/` may be read and built. Commits: no Claude/AI/assistant references, no Co-Authored-By.

**Read-first anchors (ground every task in the real code):** `packages/board-config/src/diagram-to-config.ts` (the 311-line converter being superseded — its YAML output shapes are the compatibility contract), `packages/mcp/src/diagnostics.ts` + `index.ts` (legacy 14 codes + tool registration), `packages/api/src/mcp/tools.ts` (hosted validator), `core/configs/systems/esp32s3-zero.yaml` + the spice-dispenser board config in `core/configs/` (target manifest shapes).

---

### Task 1: `compile()` kernel — ERC gate + dispatch + manifest emission

**Files:**
- Create: `packages/board-config/src/compile/index.ts`
- Modify: `packages/board-config/src/diagram-to-config.ts` (delegate)
- Modify: `packages/board-config/src/index.ts` (export)
- Test: `packages/board-config/test/compile.test.ts`

**Contract:**

```typescript
export interface CompileResult {
  ok: boolean;
  /** Present when ok. */
  systemYaml?: string;
  chipYaml?: string;
  /** All ERC findings + compile-stage findings; errors imply !ok. */
  diagnostics: Diagnostic[];
}
export function compile(input: Diagram | DiagramV2): CompileResult;
```

Behavior (TDD; write the tests below first):
1. Run `erc()`; any `severity === 'error'` → `{ ok: false, diagnostics }` (no YAML).
2. Migrate to v2, resolve nets, then dispatch over parts by `getCatalogPart(type).deviceClass`:
   - `mcu` → selects chip YAML exactly as `diagramToConfig` does today (read it; reuse its chip-yaml lookup verbatim).
   - `board_io` → emit the same `board_io` entries the legacy converter emits (preserve emission — spec: removal is a separate decision). Reuse the legacy code paths by extraction, not duplication: move the relevant generator functions out of `diagram-to-config.ts` into `src/compile/emitters.ts` and import them from both.
   - `i2c_device` → `external_devices` entry. **Bus binding is net-derived:** the device's `i2c_sda`-role pin's resolved net must contain an MCU pin whose pin-map function declares an i2c peripheral; that peripheral name (e.g. `i2c0`) becomes `connection:`. If the net contains no i2c-capable MCU pin, emit diagnostic `COMPILE_BUS_UNBOUND` (error). Address from `attrs.i2c_address`.
   - `spi_device` / `uart_device` → same legacy emission for now (extract + reuse), net-derived binding where the legacy converter already infers a peripheral; otherwise keep its existing inference and mark with a code comment `// TODO(plan-d): net-derived spi/uart binding` (allowed here because the spec scopes net-derivation to I2C-first; the TODO references a tracked follow-up, and tests cover current behavior).
   - `passive` → no manifest output (validation-only parts).
3. Parts with `type: 'ir'`-style entries: a diagram part whose `attrs.spec_path` is set and whose type is `ir` compiles to `external_devices: { type: ir, connection: <net-derived>, config: { spec_path } }`.

Tests (`test/compile.test.ts`):
- ERC-error diagram (e.g. the dispenser fixture with GND misspelled) → `ok: false`, no yaml, diagnostics contain the SCHEMA code.
- Clean dispenser fixture → `ok: true`; parse `systemYaml` with a YAML parser (add `yaml` devDependency ONLY if one already exists in the workspace — check; otherwise assert with string matching on stable lines) and assert: an `external_devices` entry with `type` matching the pca9685's legacy type mapping, `connection` equal to the i2c peripheral that owns GPIO8/9 per the esp32-s3-zero pin map, address `0x40`.
- `COMPILE_BUS_UNBOUND` when SDA net has no MCU i2c-capable pin (wire SDA to a plain GPIO with no i2c function).
- Back-compat: for the bundled v1 diagrams (pick 2 from `packages/playground/src/bundled-configs*` — read them), `compile()` output `systemYaml` is equivalent to legacy `diagramToConfig()` output (string-equal after trimming, or structurally equal if key order shifts — prefer extracting shared emitters so output is string-identical).
- `diagramToConfig()` still works (now delegating) — existing tests keep passing unchanged.

Commit: `feat(board-config): compile() — ERC-gated diagram→manifest with net-derived I2C binding`

---

### Task 2: IRQ ordinal check at compile

**Files:**
- Create: `packages/board-config/src/compile/irq-ordinals.ts`
- Modify: `packages/board-config/src/compile/index.ts`
- Test: `packages/board-config/test/compile-irq.test.ts`

The ESP-IDF `ets_isr_source_t` ordinal table for the peripherals we bind on ESP32-S3 (from core's interrupt wiring — verify against `core/crates/core/src/peripherals/esp32s3/` source or core docs in THE WORKTREE's core checkout; the I2C0=42 value is silicon-verified):

```typescript
/** ESP32-S3 ets_isr_source_t ordinals for manifest irq validation. */
export const ESP32S3_IRQ_SOURCES: Record<string, number> = {
  i2c0: 42,
  // extend from core source as verified; only entries we can verify land here
};
```

Behavior: when a compiled or user-supplied `external_devices`/`board_io` entry carries an `irq`/`irq_source` field (read the legacy converter + a core system YAML to find the actual field name; if NO irq field exists anywhere in current manifests, implement the check for diagram part `attrs.irq_source` instead and document that), validate it equals the table value for the bound peripheral; mismatch → `IRQ_SOURCE_ORDINAL` (error) with the correct value in the hint. Unknown peripheral (not in table) → no finding (table is allowlist-style).

Tests: attrs.irq_source 49 on an i2c0-bound device → error with hint containing "42"; 42 → clean; absent → clean.

Commit: `feat(board-config): IRQ source-ordinal validation at compile (ESP32-S3 table)`

---

### Task 3: Local MCP adapter — validate upgrade + compile tool

**Files:**
- Modify: `packages/mcp/src/index.ts`, `tool-metadata.ts`, `search-tools.ts`, `diagnostics.ts`
- Test: `packages/mcp/src/cli.test.ts` (or its test file convention)

1. `labwired_validate_diagram` upgrade: after the existing legacy-14 validation, ALSO run kernel `erc()` (import from `@labwired/board-config` — check how mcp currently imports board-config; if it doesn't (it has hand-copied pin maps), ADD the workspace dependency the same way ui/playground declare it — read their package.json `dependencies` for the local-path/workspace syntax) and merge diagnostics (kernel codes appended; shape-align: legacy diagnostics' shape vs kernel `{code,severity,message,hint,subjects}` — emit the union shape both ways; do not break existing consumers' field expectations — additive fields only). The stale `packages/mcp/src/pin-mapping.ts` hand-copy: replace its usage with board-config imports and DELETE the copy (this is the Plan-C consolidation; run mcp tests to prove nothing depended on its divergent content — it lacked esp32 maps entirely, so validations only get stronger; if a test asserted absence of a board, fix the test expectation with justification in the commit message).
2. New tool `labwired_compile_diagram`: input `{ diagram: object, name?: string }`; runs kernel `compile()`; on `ok:false` → tool error with diagnostics JSON (validate_diagram error style); on success persists `systemYaml` to `<repoRoot>/.labwired/boards/<kebab-name>.yaml` (same persistence/kebab/path-safety pattern as `labwired_define_component` — reuse its helpers) and returns `{ ok, board_path, system_yaml, diagnostics }` (warnings included). Title "Compile Diagram", readOnlyHint false. Register in metadata + search keywords ("compile diagram", "diagram to manifest", "build board").
3. Tests in the package's established stdio style: tool advertised; ERC-error diagram → isError with code visible; clean dispenser diagram (build it inline) → ok true, board_path matches /\.labwired\/boards\/.*\.yaml$/, system_yaml contains the i2c connection; validate_diagram on a diagram with a kernel-only failure (e.g. I2C_ADDR_CONFLICT) now reports it.

Commit: `feat(mcp): kernel-backed validate_diagram + labwired_compile_diagram`

---

### Task 4: Hosted API adapter + parity test

**Files:**
- Modify: `packages/api/src/mcp/tools.ts` (+ metadata file if separate)
- Test: `packages/api` test files (read `packages/api` test layout first; the MCP-quality work added mcp-tools.test.ts)
- Test (parity): `packages/board-config/test/surface-parity.test.ts`

1. Hosted `labwired_validate_diagram`: replace the hand-rolled 6-code validator with the same composition as local: legacy checks (port the missing 8 codes by calling the kernel + the mcp-shared legacy logic — if legacy logic lives only in packages/mcp, move the shared part into board-config `src/legacy-diagnostics.ts` first so BOTH surfaces import it; that completes the "thin adapters" spec requirement) + kernel `erc()`. Workers bundle board-config (pure TS — confirm `wrangler`/build passes).
2. Hosted `labwired_compile_diagram`: same kernel call; hosted persistence: return `system_yaml` inline only (no filesystem on Workers) — document the difference in the tool description.
3. **Parity test** (spec requirement): in board-config (neutral ground), a fixture set (clean dispenser + one fixture per rule family) run through BOTH surfaces' diagnostic composition functions — import the composition functions from mcp and api packages if importable; if cross-package imports are awkward, place the shared composition itself in board-config (`composeDiagnostics(diagram)`) and have both adapters call it — then the parity test asserts both adapters delegate to it (import-level parity, simpler and stronger). Prefer the latter.
4. `packages/api` tests + build green; existing auth/metadata tests untouched.

Commit: `feat(api): hosted validate/compile on the shared wiring kernel (surface parity)`

---

### Task 5: Hero golden + full verification + docs

**Files:**
- Create: `packages/board-config/test/dispenser-golden.test.ts`
- Modify: `packages/mcp/src/resources/labwired-agent-hardware-loop.md` + `packages/api/src/mcp/resources/labwired-agent-hardware-loop.md`
- Modify: `packages/mcp/README.md`, `CHANGELOG.md`

1. **Hero golden:** the dispenser diagram's `compile()` output runs against the worktree core: write the emitted systemYaml + chipYaml to a temp dir, then execute `core/target/debug/labwired run --firmware <none-needed?>`— actually use the cheapest real check: `labwired asset validate` / config-load path. Read `core/crates/cli` for a command that loads a system manifest without firmware (e.g. `labwired machine info` or the test-runner's config validation). If none exists headlessly, fall back to: structurally compare the compiled manifest against the hand-written spice-dispenser system config in the worktree's `core/configs/` (find it: `grep -rl pca9685 core/configs/`) — same external_devices entries (type/connection/address), same board_io semantics — assert key-by-key with explanatory messages. Either way the test must FAIL if compile() output stops being loadable/equivalent.
2. Agent guide: add a "Compile your diagram" step (validate → compile → run with the returned board path) to both guide copies; README tool table + status bullet; CHANGELOG entry: "Wiring kernel: named nets, electrical rule checks (ERC), and ERC-gated diagram compilation on both MCP surfaces."
3. **Full verification** (paste summaries): board-config vitest+typecheck; mcp tests+build (LABWIRED_CLI from worktree core); api tests+build; `packages/ui` build then `packages/playground` build (in that order — playground needs ui's dist).

Commits: `test(board-config): dispenser hero golden vs core config` then `docs: wiring kernel surfaces — guide, README, changelog`

---

## Self-review notes

- Spec §5 coverage: ERC-gated compile + dispatch (Task 1), net-derived I2C binding (Task 1), IR-part compilation (Task 1 item 3), board_io preserved (Task 1), IRQ ordinal (Task 2), both surfaces upgraded + compile tool + persistence (Tasks 3-4), parity via shared composition (Task 4), hero golden + guide/docs (Task 5). SPI/UART net-derivation explicitly deferred with tracked TODO (spec scopes I2C-first via the bus-binding example; flagged honestly).
- The mcp stale pin-map deletion (Plan A carry-over #5) lands in Task 3.
- Check-at-implementation anchors are explicit; YAML-parsing test strategy conditional on existing deps; hosted persistence difference documented.
- Type consistency: CompileResult/compile in compile/index.ts; emitters shared via extraction; composeDiagnostics in board-config consumed by both adapters.
