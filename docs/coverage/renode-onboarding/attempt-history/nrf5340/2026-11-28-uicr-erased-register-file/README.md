# nRF5340 UICR serves the erased SVD register file (2026-11-28)

## Change

The nRF5340 chip profile mapped UICR at `0x00FF8000` to `nrf5340_uicr_stub`,
which returned zero for every read. The chip now uses the generic `declarative`
model with the existing embedded descriptor
`configs/peripherals/nrf5340/uicr_s.yaml` (967 registers from the vendored
Nordic SVD). The stub entry was removed from `known_stubs.rs`.

UICR is non-volatile memory. An unprogrammed word reads `0xFFFFFFFF`. In the
SVD, 964 of the 967 registers have that reset value. The other three,
APPROTECT, SECUREAPPROTECT and ERASEPROTECT, have reset value `0`. The SVD's
own enums decode `0` as "Protected". A zero window, or the raw SVD reset value,
therefore claims that ERASEALL is protected on a blank part. That is false.

To correct this, the declarative factory (`crates/core/src/bus/from_config.rs`)
accepts a generic optional `reset_overrides: { REGISTER_ID: value }` mapping.
It uses the existing `GenericPeripheral::force_register_value`, so the value
persists through reset. An unknown register id or a non-32-bit value is a
config error. The nRF5340 profile overrides only the three protect words, to
`0xFFFFFFFF`.

Decoded against the SVD enums, the erased part now reads:

- APPROTECT and SECUREAPPROTECT: not `0x50FA50FA` (Unprotected), so debugger
  access is protected.
- ERASEPROTECT: `0xFFFFFFFF`, which is the SVD Unprotected value.

## Evidence

- `--test nrf nrf5340_` (`--features event-scheduler`): **14 passed**,
  including the three new `nrf5340_uicr` tests (`test-output.log`).
- Negative control (`negative-control-no-overrides.log`): with only the
  `reset_overrides` block removed, `uicr_protect_words_read_erased_not_the_svd_zero`
  fails (`left: 0`, `right: 4294967295`). The rejection test also fails because
  no mapping is present. The OTP/KEYSLOT test passes on the SVD values alone.
  It would fail on the old zero stub.
- `firmware_survival::test_nrf5340_zephyr_survival`: **1 passed**. Unmodified
  Zephyr v3.7 `nrf5340dk/nrf5340/cpuapp` boots and prints its banner with the
  erased UICR (`zephyr-survival.log`).
- `model_paths_drift_gate`: **1 passed** (`model-paths.log`); `svd_conformance`:
  **2 passed** (`svd-conformance.log`); `--test bus unknown_peripheral_type`:
  **7 passed** (`unknown-type-registry.log`).
- `generate_firmware_exercise_matrix.py --check` and
  `generate_model_ratings.py --check` passed after regeneration.

The Renode onboarding scoreboard did not change: **245 imported descriptors;
231 booted; 231 booted and smoke-passing; 14 `not_run`; 0 failures among
attempted runs**. nRF5340 is not one of the 245 imported descriptors. The
aggregator was not rerun.

## Limits

- Writes store directly into the register file. NVMC `CONFIG.WEN` gating,
  1 → 0 flash-program semantics and `ERASEUICR` are not modelled (no nRF5340
  NVMC is mapped in this profile).
- Reserved gaps between SVD registers are not claimed to read erased.
- No silicon capture. Not a model of a provisioned device.
