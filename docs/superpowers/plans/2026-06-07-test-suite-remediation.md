# Test-Suite Remediation Plan

**Date:** 2026-06-07
**Input:** three-part test audit (core/crates-core, core/other-crates, parent JS/TS), ~1,980 tests.
**Finding:** the suite's core is strong; the failure pattern is *tests whose preconditions CI never satisfies but which still count green* — disarmed gates, silent skips, feature-gated tests no workflow compiles, and whole packages with zero CI.

## Scope decisions (owner-approved)

- **Parked until HIL:** anything needing silicon — making F103/L476 MMIO-diff
  divergences unconditionally fatal, re-capturing silicon goldens for the
  simulator-produced F407 references, populating hardware-oracle captures.
  Tracked here, not actioned. (We don't run silicon validation per-PR today;
  that arrives with the HIL bench workstream.)
- **Approved lane 2:** new CI lanes — host-only feature lanes (`jit`,
  `event-scheduler`) in core-integrity; espup nightly lane for Xtensa-fixture
  e2e tests; CI coverage for board-config / builder / mcp in the parent repo.
- **Approved lane 3:** delete true corpses; fix weak assertions opportunistically.

## Work items

### Core repo — code (no hardware needed)
1. **Arm the SVD coverage ratchet**: vendor the Espressif `esp32s3.svd` (MIT-licensed,
   espressif/svd) under `tests/fixtures/svd/`, point `discover_svd()` fallback or CI
   env `LABWIRED_ESP32S3_SVD` at it. The ratchet has never executed in CI.
2. **Delete corpses**: `test_arm_bfi_bfc` (cortex_m.rs:2343, zero asserts),
   `test_blinky_perf_throughput_50mips` (integration.rs:1978, print-only),
   `test_dap_evaluate_register` (dap/tests/evaluation.rs:5, builds+discards),
   `test_nested_irq_config_validation` (integration_stress_tests.rs:81, admits
   it doesn't test IRQs), `aht20_bmp280_chip_id_handshake_matches_silicon` +
   its empty fixture (e2e_stm32f407_i2c.rs:242 — `events: []`; leave a tracking
   comment for the HIL re-capture).
3. **Make silent skips visible/armed**:
   - dap e2e + loader `location_to_pc`: add the missing debug fixture build to
     core-ci's fixture step (`firmware-ci-fixture` thumbv6m debug) so they RUN.
   - `e2e_esp32_epaper`, `runtime_snapshot` agentdeck, `jit_lockstep` quiet-skips:
     convert silent `return` to `#[ignore]` with honest reason strings (armed by
     the nightly espup lane where applicable).
   - `peripheral_kit_gate::manifest_json_matches_registry`: vendor the TS manifest
     copy in-repo so the drift gate fires in CI.
4. **Strengthen weak asserts** (audit B-lists): h5_demo UART assertion; brom smoke
   minimum-instructions assert; golden_examples content asserts (stop_reason +
   uart content); snapshots phase-2 PC-resume assert; determinism baseline pin;
   asset_import structure asserts; VCD event assert; interactive_snapshot PC/SP
   range asserts; gdb single-step PC-advance assert; event_scheduler exact-count
   assert; firmware_survival name-based case lookup (kill index drift);
   strict_onboarding shrink-only smoke-less allowlist.

### Core repo — CI lanes
5. core-integrity: add host-only `cargo test -p labwired-core --features jit,event-scheduler`
   step (pure unit tests, no toolchain).
6. core-nightly: espup lane building the `esp32s3-fixtures` e2e firmware and running
   `cargo test -p labwired-core --features esp32s3-fixtures` + selected `--ignored`
   suites it can satisfy.

### Parent repo
7. playground-ci: run board-config tests (+ mcp tests on PR, not only on release tags).
8. builder: skip-guard `run.test.ts` on missing binary (visible skip, not ENOENT);
   add a workflow job that builds the core CLI then runs builder tests.
9. wasm crate has ZERO tests (2,500-line API) — separate workstream, too big for
   this remediation; tracked as a gap.

## Verification
Each item lands with the gate it arms actually failing when sabotaged (one-off
local check), then green. No gate may be weakened to pass.
