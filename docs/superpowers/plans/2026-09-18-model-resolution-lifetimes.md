# Model resolution and lifetime repair

**Goal:** A YAML I²C device has identical identity, configuration and behavior directly on a controller or behind a mux; loading edited runtime parts does not retain their allocations forever.

**Authorization:** The user approved the architecture review and requested implementation on 2026-09-18.

**Architecture:** Resolve manifest overrides before embedded descriptors and Rust fallbacks. Topology validation and tree construction share the leaf resolver. Dynamic descriptors and metadata belong to their runtime owners; static built-in kits remain reusable. Preserve unsupported-type errors and explicit override requirements.

**Stack:** Rust, existing descriptor schema, Cargo integration tests, native and wasm builds.

## Ownership contract

Runtime-authored descriptors must not enter a process-lifetime interning table.
`PeripheralKit::metadata()` and `SimInput::input_channels()` borrow their owner;
discovery snapshots clone their channel descriptions when they must outlive it.
Metadata strings and tables use `Cow` so built-in literals remain borrowed while
runtime YAML owns its allocations. This changes Rust API ownership, not the
serialized manifest/discovery schema. Rust callers that previously copied an
`InputChannel` must clone it; callers inspecting metadata should borrow fields.
The built-in registry may own a fixed, lazily initialized collection of kits,
but loading or editing a manifest must not grow that collection.

## Resolver

- [x] Establish baseline with `cargo test -p labwired-core --test i2c_mux_tca9548a --test part_pack_contract --test i2c_factory_kit_coverage` (36 passed).
- [x] Add `crates/core/tests/yaml_mux_resolution.rs`: every embedded I²C descriptor and manifest-carried sensors must validate and execute behind nested muxes. Check configuration seeding, identity, overrides, wrong transports and address validation.
- [x] Run the original eight regression cases before production changes (eight failed); confirm the two additional address/mux override cases fail before fixing them.
- [x] Update `crates/core/src/peripherals/components/i2c_factory.rs` to use descriptor discovery instead of the per-device YAML allowlist; share leaf resolution with topology validation.
- [x] Ensure `crates/core/src/bus/part_pack.rs` uses the same configuration seeding as built-ins and preserves override precedence. Move noise-knob seeding into the shared model method after reproducing its failure behind a mux.
- [x] Run existing mux, pack, factory and migrated-device contract tests: 163 tests passed across 20 integration suites, including all 13 new resolver regressions and the unchanged catalog-byte comparison.

## Runtime ownership

- [x] Trace static kit/channel contracts and write an allocation-retention regression for repeated distinct runtime packs (all 12 tested constructor cases retained memory before the fix).
- [x] Remove permanent interning for dynamic packs and ensure channels/metadata have an owned lifetime. All core unit/integration test targets compile after adapting Rust consumers.
- [x] Verify drop/reload behavior, edited definitions with the same type, discovery and input routing: the allocation regression passes with zero retained bytes across every measured constructor/bus case, including failed attachment.

## Verification and handoff

- [x] Review spec compliance and code quality independently (both inspections passed; runtime verification remains required).
- [x] Run formatting, relevant core suites, and wasm compilation; report exact results and pre-existing warnings below.
- [x] Record remaining product work separately: richer drafting/driver onboarding, fidelity upgrades, provenance-based coverage, and performance work. Do not claim those delivered by a resolver repair.

## Remaining product work (not delivered here)

- Expand drafting from basic register skeletons into rules/timers/output bindings
  with a driver-backed verification loop.
- Upgrade device fidelity where YAML migration currently preserves stubs (for
  example BMP280 stimulus and BMI270 FIFO/interrupt/time behavior).
- Replace filename-derived YAML/Rust coverage counts with runtime provenance.
- Continue the independent real-time/browser and RISC-V execution workstreams.

This repair neither changes hardware-validation claims nor proves simulator
wall-clock performance. It does not update the app's core submodule pin or
deploy browser assets.

## Verification evidence

- `cargo check -j 2 -p labwired-core --tests`: passed (all core test targets compile).
- Targeted runtime verification: 163 tests passed in 20 integration suites.
  This includes 13 mux-resolution regressions, zero retained allocations across
  the measured runtime reload/drop cases, and byte-identical catalog JSON against
  the unchanged committed fixture.
- `cargo clippy -j 2 -p labwired-core -p labwired-cli --all-targets -- -D warnings`:
  passed after removing one needless borrow in the new allocation test.
- After that test-only correction, the allocation test and all 13 resolver
  regressions were rerun together: 14 passed, zero failures.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo check -j 2 -p labwired-wasm --target wasm32-unknown-unknown`:
  passed. Six warnings remain in unchanged virtual Wi-Fi and GDB stub code
  (unused native-only imports and a dead constant on the wasm target).

Cargo checks/tests above use local command-line overrides
`--config 'profile.dev.package.labwired-core.opt-level=0'` and
`--config 'profile.dev.package.labwired-core.debug=0'` to limit build cost on a
busy host. No repository build profiles were changed. These are correctness
checks, not release-mode performance measurements.
