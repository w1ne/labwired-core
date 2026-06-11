# LabWired wiring kernel design (Slice 2)

Date: 2026-06-12
Status: Approved for implementation
Scope: Make wiring robust enough that agents build real hardware against it — one wiring kernel (schema v2 with first-class nets, typed pins, ERC, diagram→manifest compile) consumed by every surface.
Parent: `2026-06-11-hw-substrate-sota-design.md` (this supersedes and deepens its "Slice 2 — Construction surface completion" section).

## Why this shape

Audit of the current model (2026-06-11): four semi-independent schema definitions (board-config, ui, mcp, api); the hosted validator implements 6 of 14 diagnostic codes; pin maps for only 5 MCUs are hand-copied into TypeScript while core's chip YAMLs go unused; wires are typeless point-to-point pairs with implicit nets; no power/voltage/pull-up/address/CS validation; no schema version; `diagram-to-config.ts` works but is a tangle of 8 device special cases; `board_io` is emitted but never consumed by core.

Comparables audit (primary sources, 2026-06-12): Wokwi's `part:pin` connection tuples and per-part `attrs` are the right authoring primitives, but its implicit nets and type-blind pins permit no static validation. KiCad contributes the two most valuable artifacts: named nets as first-class objects and the 12-type pin electrical vocabulary with its ERC conflict matrix. Renode's `device @ bus address` registration is the right abstraction for protocol-addressed devices. Fritzing (form-factor-only connectors, implicit nets) and SPICE (full analog simulation) are anti-models for this use case.

Decisions locked with the user: named nets canonical with wires as compatible sugar; ERC depth = protocol + power correctness (no current/drive budgets); surfaces in scope = kernel + local MCP + hosted API, UI read-only compatible.

## Section 1 — One wiring kernel in `@labwired/board-config`

All wiring knowledge consolidates into `packages/board-config` (already the least-duplicated package; `packages/ui` re-exports from it). Six focused modules, each independently testable:

- `schema` — Diagram v2 types, version field, pure lossless v1→v2 migration.
- `catalog` — the single declarative part catalog: every part type declares its pins (typed, see Section 3), device class, and attrs schema. Replaces both `packages/mcp/src/component-meta.ts` and the registry duplicated in `packages/ui/src/editor/components/index.ts` (UI keeps its render components; their metadata moves here).
- `pins` — MCU pin maps stay declaratively authored in board-config as the **single canonical source**, extended with electrical types and `internal_pullup` capability. (Amended 2026-06-12: the original intent was codegen from `core/configs/chips/*.yaml`, but those YAMLs carry peripherals and memory maps only — no pin tables exist in core today. Upstreaming pin tables into core YAML and generating from them is future work, see Non-goals.) A diagram referencing a board absent from the maps is a diagnostic, never a silent fallback.
- `normalize` — deterministic wires→nets closure (stable net naming derived from sorted member pins; same input always yields same nets).
- `erc` — the rule engine of Section 4. Pure function: `erc(diagramV2, catalog, pinMaps) → Diagnostic[]`.
- `compile` — Section 5. Pure function returning manifest YAML + diagnostics.

Local MCP (`packages/mcp`) and hosted API (`packages/api`) become thin adapters over these functions. The kernel is pure TypeScript with no runtime dependencies, so the Cloudflare worker bundles it directly. Validation parity across surfaces becomes structural, not aspirational.

## Section 2 — Diagram schema v2

```jsonc
{
  "version": 2,
  "parts": [ { "id": "pca1", "type": "nxp/pca9685", "attrs": { "i2c_address": "0x40" } } ],
  "nets": [
    { "name": "GND",      "kind": "power",  "voltage": 0 },
    { "name": "3V3",      "kind": "power",  "voltage": 3.3 },
    { "name": "I2C0_SDA", "kind": "signal", "protocol": "i2c_sda" },
    { "name": "I2C0_SCL", "kind": "signal", "protocol": "i2c_scl" }
  ],
  "connections": [
    ["esp1:GPIO8",  "I2C0_SDA"],
    ["pca1:SDA",    "I2C0_SDA"],
    ["r1:1",        "I2C0_SDA"],
    ["r1:2",        "3V3"]
  ],
  "wires": [ /* legacy point-to-point pairs, optional */ ]
}
```

- Parts keep the Wokwi-shaped `{id, type, attrs}`; `attrs` carries parametrics (I2C address, resistance, color).
- Nets are first-class: `kind: signal | power`, optional `voltage` (power) and `protocol` (signal). Net names are unique.
- Connections bind `partId:pinName` to a net name. Multi-instance pins disambiguate with a `.N` suffix (`esp1:GND.2`).
- Legacy `wires` (`{from: {part, pin}, to: {part, pin}}`) remain accepted. The normalizer folds wires and connections into one resolved net set before validation/compile; wire-only diagrams (every existing v1 diagram, the UI editor, watch sessions) keep working unchanged. Versionless input is treated as v1 and migrated. Migration is pure and lossless; v2 is the only form validate/compile operate on internally.
- No routing geometry in the kernel schema. UI geometry (x/y/waypoints/color) lives in the UI layer's extension of the type, ignored by the kernel.

## Section 3 — Typed pins and the part catalog

Pin electrical types use the KiCad vocabulary: `input, output, bidirectional, tri_state, passive, open_drain, open_emitter, power_in, power_out, nc, unspecified, not_internally_connected`. Catalog pins are `{ name, etype, role? }` where `role` carries protocol meaning: `i2c_sda, i2c_scl, spi_mosi, spi_miso, spi_sck, spi_cs, uart_tx, uart_rx, pwm, adc, gpio, irq`. Examples: PCA9685 SDA = `{etype: open_drain, role: i2c_sda}`; resistor pins = `passive`; VCC = `power_in`; an MCU 3V3 pin = `power_out`.

MCU pins in the generated maps carry `etype` plus the existing capability set (`gpio/adc/i2c/spi/timer/uart` with peripheral + role), extended with `internal_pullup: bool` where the chip supports it. Required-input pins (e.g. a sensor's address-select that must be strapped) are declared `{required: true}` in the catalog so floating-input detection has ground truth.

The catalog is declarative data (TS objects with a JSON-stable shape), versioned with the package. IR-defined components (`labwired_define_component`, Slice 1) map into catalog entries automatically: the IrComponent interface section provides the pins (I2C → SDA/SCL open-drain + VCC/GND power_in), so agent-defined parts are first-class citizens of validation.

## Section 4 — ERC rule set

The existing 14 diagnostic codes keep their exact names, severities, and semantics — they are a published contract. New rules, every one machine-readable `{code, severity, message, hint}`:

**Pin-pair matrix** (adapted from KiCad's 12×12, evaluated per net over all pin pairs):
- `NET_DRIVER_CONFLICT` (error) — two push-pull outputs, or output + power_out, on one net.
- `NET_RAIL_SHORT` (error) — two power_out pins on one net, or two power nets with different voltages bridged by shared members.
- `NET_NC_CONNECTED` (error) — a `nc` pin appears in any net.
- `NET_UNSPECIFIED_PIN` (warning) — `unspecified` etype meets any typed pin.

**Power:**
- `PWR_RAIL_UNDRIVEN` (error) — net with `power_in` pins but no `power_out` member.
- `PWR_VOLTAGE_MISMATCH` (error) — part operating range (catalog) excludes the voltage of the power net feeding its `power_in` pin.
- `PWR_NO_GROUND` (warning) — a powered part with no pin on a 0 V net.

**Buses:**
- `I2C_ADDR_CONFLICT` (error) — two devices with equal resolved 7-bit address whose SDA/SCL pins share nets.
- `I2C_NO_PULLUP` (warning) — open-drain net (i2c_sda/i2c_scl) with neither a passive part path to a power net nor an MCU pin with `internal_pullup` enabled via attrs.
- `SPI_NO_CS` (warning) — SPI device whose `spi_cs` pin is on no net driven by an MCU output.
- `UART_CROSSOVER` (error) — uart_rx↔uart_rx or uart_tx↔uart_tx pairing.

**Integration:**
- `IRQ_SOURCE_ORDINAL` (error, at compile) — a device IRQ binding whose source id differs from the ESP-IDF `ets_isr_source_t` ordinal for the bound peripheral (the I2C0=42-not-49 class of silent never-dispatched ISR).
- `PIN_INPUT_FLOATING` (warning) — catalog `required: true` input pin on no net.

Severity philosophy: errors are things that brick or damage real hardware or firmware bring-up; warnings are protocol-correctness risks with plausible legitimate exceptions. `compile` refuses on errors, proceeds with warnings (returned alongside output).

## Section 5 — Compiler, surfaces, board_io

`compile(diagramV2) → { systemYaml, chipYaml, diagnostics }`. Rebuilt as a dispatch table keyed by catalog device class (mcu, board_io, i2c_device, spi_device, uart_device, passive), replacing `diagram-to-config.ts`'s 8 inline special cases. ERC errors abort compile (diagnostics returned, no YAML). Bus binding follows the Renode-style derivation: a device whose i2c_sda/i2c_scl pins share nets with MCU pins of a specific I2C peripheral binds to that peripheral (`connection: i2c0`) with its resolved address — the net topology, not a hand-authored field, determines the binding. IR components compile to `type: ir` + `spec_path` entries.

`board_io` entries continue to be emitted exactly as today until the implementation plan inventories their real consumers (core ignores them; the UI/watch path may not) — removing them is an explicit follow-up decision, never a silent break.

Surfaces: `labwired_validate_diagram` upgraded on local MCP **and** hosted API (same kernel call, full code set); new `labwired_compile_diagram` on both, returning the manifest + diagnostics with the same persistence pattern as `define_component` (`.labwired/boards/<name>.yaml`, plus the run-tool usage snippet). The agent loop becomes: define parts → compose nets → validate → compile → run firmware, with no hand-authored YAML.

## Error handling

- All kernel functions are pure and total: malformed input yields diagnostics (`SCHEMA_*` codes), never throws across the adapter boundary.
- Unknown chip, unknown part type, unparseable version → distinct diagnostics with hints (closest-match suggestion for part-type typos).
- Migration never fails: any v1-shaped diagram produces a v2 with synthetic nets; ERC then judges it.

## Testing

- **Matrix tests**: table-driven over every pin-pair cell (the adapted matrix is data; the test iterates it).
- **One fixture per ERC rule**, mutation-style: a minimal diagram that triggers the rule, and its corrected twin that must pass — both committed.
- **Migration round-trips**: every existing bundled diagram (playground bundled-configs, examples) migrates v1→v2, validates without new errors, and compiles to a manifest equivalent to the current converter's output (goldens taken from current behavior where current behavior is correct).
- **Hero fixture**: SpiceDispenser wiring — ESP32-S3 + PCA9685 + two servos + 3V3/5V/GND rails + I2C pull-ups — validates clean, compiles to a manifest equivalent to the hand-written `spice-dispenser` board config, and every ERC rule has been exercised by deliberately breaking this fixture in a parameterized test (remove pull-up, conflict the address, drop GND, float CS…).
- **Surface parity test**: the local MCP and hosted API validators run the same fixture set and must return identical diagnostic code sets.
- Full gates: board-config + mcp + api test suites, builds, plus playground `vite build` (submodule-pin/imports regression caught in Slice 1 review).

## Non-goals

- Pin-table codegen from core chip YAMLs (blocked: core YAMLs carry no pin data; upstreaming pin tables into core is a separate workstream).
- UI editor net-authoring UX (rail bars, net inspector) — UI stays read-compatible only.
- Core-side pin mux / GPIO matrix modeling.
- Current/drive-strength budgeting and analog behavior.
- Removing `board_io` (inventory first; separate decision).
- New device models (Slice 1 owns components; this slice owns the wiring between them).
