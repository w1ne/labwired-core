# nRF5340 FICR serves the vendor SVD register file (2026-11-28)

## Change

The nRF5340 chip profile mapped FICR at `0x00FF0000` to
`nrf5340_ficr_stub`, which returned zero for reads. The existing embedded
`configs/peripherals/nrf5340/ficr_s.yaml` descriptor already contains the
vendored Nordic SVD register layout, offsets, access modes, and reset values.
The chip now uses the generic `declarative` model for that register file, and
the zero stub was removed from the allowlist.

The implementation preserves what the SVD actually specifies:

- `INFO.PART` is `N5340` (`0x00005340`), and `DEVICETYPE` is the SVD reset
  value.
- `CODEPAGESIZE` × `CODESIZE` is 4 KiB × 256 pages, matching the chip profile's
  1 MiB flash.
- Device IDs, variant, package, RAM/FLASH variant, and the trim table stay at
  their SVD unspecified value (`0xFFFFFFFF`). No production-lot identity or
  calibration data is invented.

## Evidence

- `nrf5340_ficr` register tests: **3 passed**. The checks read the values from
  the bus, compare selected register offsets and reset values with the vendored
  SVD, check flash geometry, preserve unspecified fields, and verify
  `INFO.PART` is read-only.
- `firmware_survival::test_nrf5340_zephyr_survival`: **1 passed**. The existing
  unmodified Zephyr v3.7 `nrf5340dk/nrf5340/cpuapp` ELF booted and printed its
  banner after the FICR mapping change.
- `model_paths_drift_gate::every_registered_model_is_in_its_board_models_list`:
  **1 passed**; the board watch list now includes `declarative.rs` and the
  FICR descriptor.
- `svd_conformance`: **2 passed**.
- `generate_firmware_exercise_matrix.py --check` and
  `generate_model_ratings.py --check` passed after regenerating their outputs.

The full onboarding scoreboard did not change: **245 imported descriptors;
231 booted; 231 booted and smoke-passing; 14 `not_run`; 0 failures among
attempted runs**. nRF5340 is not one of those 245 imported descriptors, so its
Zephyr run is board evidence and is not added to those inventory counts. No
onboarding metrics changed; the scoreboard aggregator was not rerun.

Logs: `test-output.log`, `zephyr-survival.log`, `model-paths.log`, and
`svd-conformance.log`.

## Limits

This is a declarative register file, not a silicon capture or a model of
per-device factory programming. The separate network core, radio, TrustZone
enforcement, and analog trim values remain unmodeled.
