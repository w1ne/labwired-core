# Hosted verifier attempt

The hosted catalog currently offers `esp32c3-supermini` as a runnable ESP32-C3
board, not the exact ESP32-C3-DevKitM-1-N4X. Its diagram validator accepted a
generic C3 plus one NeoPixel wired to GPIO8, 3V3, and GND.

The hosted verifier reports that the C3 Super Mini supports `arduino` and
`rust`, but its published `entryPath` schema omits `src/main.ino`. Passing the
required Arduino entry path returned `INVALID_ARGUMENT`; omitting it returned
`INVALID_ENTRY_PATH` (`entryPath must be src/main.ino for language arduino`).
Trying C++ returned `LANGUAGE_NOT_SUPPORTED` for this board. No hosted
`model_verified` verdict was produced.

The exact board profile is covered by the committed repository CLI smoke in
`logs/run_smoke.log` and `simulation-output/result.json`; those are local model
evidence, not a hosted verifier verdict or physical-board confirmation.
