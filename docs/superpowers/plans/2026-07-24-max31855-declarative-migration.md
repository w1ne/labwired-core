# MAX31855 → declarative migration plan

> Executes with superpowers:subagent-driven-development once approved. Destructive/cross-cutting — **do not start until the user approves this plan.**

**Goal:** Make the declarative `spi_device` primitive the single source of truth for the MAX31855, removing the hand-written per-part Rust while preserving every externally observable behavior (byte-exact SPI frames, wasm live-inspection, canvas emit, the demo lab).

**Why it's big:** the MAX31855 currently has THREE overlapping models + several consumers coupled to the concrete Rust type:
- `crates/core/src/peripherals/components/max31855.rs` — hand-written `Max31855` + `Max31855Kit` (device_type `"max31855"`, carries a `LabRef` to `max31855-thermocouple-lab`).
- `configs/components/max31855.yaml` + `ir_spi_component.rs` — an IR component representation with an **equivalence gate** (`ir_spi_matches_handwritten_max31855`) that clocks it byte-for-byte against the hand-written type across default/positive/negative/**fault**/saturation cases.
- `configs/devices/max31855_spi.yaml` — the new declarative descriptor (Phase-2 Task 4), currently registered as `max31855_spi` to avoid the device_type collision.
- Consumers of the concrete type: `crates/wasm/src/inspect.rs` (downcasts to `Max31855`, reads `temperature()` → `tc_c`/`internal_c`), `canonical.rs` `SPI_DEVICE_TYPES` whitelist + emit fixtures, `ir_spi_component.rs` test oracle.

## Parity anchor (must hold byte-for-byte)

Hand-written default word = `(100<<18)|(352<<4)` = `0x01901600` — tc=25.0 °C (×4=100), internal=22.0 °C (×16=352), fault=0. SimInput channel keys are **`temperature`** (hot junction) and **`internal`**. The declarative descriptor MUST reproduce these exactly:
- Fix `max31855_spi.yaml` defaults+keys: field source keys → `temperature` (default **25**) and `internal` (default **22**, not 25); input `key`s likewise `temperature`/`internal`. Scales already correct (4.0 / 16.0).
- Fault is NOT a SimInput channel on the hand-written model (only a public struct field, unreachable through the kit), so a fault-free declarative model is byte-identical for every kit/SimInput-reachable path. Documented as such.

## Tasks

### Task 1 — Descriptor parity (non-destructive)
Rename `configs/devices/max31855_spi.yaml` → `max31855.yaml`; set `type: max31855`; field sources `temperature`/`internal`, input keys `temperature`/`internal`, internal default `22`. Update the header comment (drop the collision note; document fault-free + key rationale). Do NOT register yet (still colliding). Add a **parity unit test** next to the declarative engine that asserts the descriptor's default frame == `0x01901600` and the vectors `(400,400)`→`(100<<18)|(400<<4)` and negative `(-100,296)`. Gate: `cargo test -p labwired-core max31855_parity`.

### Task 2 — Generic temperature accessor
Add `pub fn input_value(&self, key: &str) -> Option<f64>` to `GenericSpiDevice` (reads `self.slots`). Unit-test it. This is what the wasm inspector and any future generic consumer read instead of a concrete downcast.

### Task 3 — Rework wasm inspector
In `crates/wasm/src/inspect.rs`, replace the `downcast_ref::<Max31855>()` + `sensor.temperature()` path with: downcast the attached device to `GenericSpiDevice`, read `input_value("temperature")` / `input_value("internal")`. Keep the emitted JSON shape identical (`{id, kind:"max31855", tc_c, internal_c}`). Gate: `cargo build -p labwired-wasm` (or the crate's check) + any inspect test.

### Task 4 — Repoint the IR equivalence gate
`ir_spi_component.rs::ir_spi_matches_handwritten_max31855` uses `Max31855` as its oracle. Replace the oracle with the **explicit datasheet word formula** (`(tc_q14<<18)|(fault<<16)|(int12<<4)`, masked per field) already documented in that test's cases, so the IR component is still gated without depending on the deleted type. (The IR `configs/components/max31855.yaml` path is independent of the declarative kit and stays.) Gate: `cargo test -p labwired-core ir_spi`.

### Task 5 — Delete hand-written model, register declarative as `max31855`
Delete `components/max31855.rs`; remove its `mod`/`pub use` in `mod.rs` and its `MAX31855_KIT` line in `registry.rs`. Register the declarative kit as device_type `max31855`: embed key `max31855`, `MAX31855_KIT` static, `registry::KITS` line. **Lab preservation:** the declarative `KitMetadata.labs` is `&[]`, so the manifest loses the `max31855-thermocouple-lab` association. Options (pick in review): (a) accept the loss (example dir stays on disk, still runnable, just not advertised in the manifest) and note it; or (b) teach `DeclarativeSpiKit`/descriptor to carry a `labs` ref (larger). Regenerate the manifest. Grep for any remaining `Max31855`/`"max31855_spi"` references and clean them. Gate: `cargo test -p labwired-core max31855 peripheral_kit_gate`.

### Task 6 — Canvas emit + full gate
Confirm `canonical.rs` `SPI_DEVICE_TYPES` still lists `max31855` (type name unchanged ⇒ emit + `EXPECTED_SPI_MAX31855` fixture unchanged; verify the emit test passes). Full suite `cargo test -p labwired-config -p labwired-core`, `cargo build -p labwired-wasm`, `cargo clippy … -D warnings`, fmt. Confirm no lingering reference to the deleted type anywhere (`grep -rn "Max31855" crates/`).

## Risk / rollback
Each task is independently revert-able; the destructive Task 5 comes only after parity (Task 1) and all consumers (Tasks 2–4) are migrated and green. The equivalence gate (Task 4) and parity test (Task 1) together prove the declarative model matches the deleted one byte-for-byte.

## Open decision for review
Lab association (Task 5): accept manifest-level loss of the one-click `max31855-thermocouple-lab`, or invest in declarative `labs` support?
