# Arduino MKR WiFi 1010 validation

This round validates the focused board profile with compiled firmware, the
local LabWired CLI and an edge-count assertion on the onboard PA20/D6 LED.
Re-run with the command in `README.md`; evidence is stored under `evidence/`.
The build used `CARGO_BUILD_RUSTC_WRAPPER=` because the runner's global Cargo
config points at an `sccache` executable that is not installed.

Scope: SAMD21 reset boot, PM.APBBMASK.PORT enable, PORTA PA20 output and the
board's active-high yellow D6 LED. The NINA-W102 radio module, RGB indicator,
ATECC508, PMIC, USB and all other board/chip peripherals are unmodeled in this
profile and are not covered by this smoke.

The `SamPm` model's APBBMASK reset value is zero, while the vendor descriptor
lists 0x7F. This firmware writes APBBMASK before touching PORT, so the exercised
GPIO clock-gate sequence does not rely on either reset value.
