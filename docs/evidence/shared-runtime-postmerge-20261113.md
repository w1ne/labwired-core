# Shared circuit runtime — post-main integration check, 13 November 2026

## Source boundary

This note records targeted checks of the conflict-resolved integration branch after merging core main `cfe553f69` into prior PR head `f2beabb29`, resulting in integration merge `761a8a351`. It does not claim physical drone flight or certify a stable production release.

- `cargo check -p labwired-core --locked --offline`: PASS.
- `cargo test -p labwired-core --test bmi270_spi_protocol --test device_supply_rails --locked --offline`: 33 passed, 0 failed.
- `cargo test -p labwired-core --test circuit_bldc --test world_circuit_bldc --test motor_models --test analog_motor_circuit --locked --offline`: 55 passed, 0 failed.
- `cargo fmt --all -- --check`: PASS at resolved code identity.

The merge preserves new circuit/rail atomic publication, the existing BMI270 SPI select and reset behavior, the native output-stage timer path, and explicit invalid integration observations. No benchmark threshold, waiver, test assertion or CI rule was changed.

## Remaining acceptance

The previous remote board-throughput run failed its required performance limits: ESP32 +3.2%, RA6M5 +3.7%, RA8M1 +3.5% relative to established per-chip baselines. This post-main merge has **not** passed a new authoritative performance run. The previous `perf-scope` run used an outdated PR base SHA in its GitHub event and rejected the synthesized merge parent identity. The normal current-base PR workflow must check the exact new source identity; do not bypass its parent check.

Application native five-MCU cold startup, motor commutation, synchronized IMU/analog output, closed-loop hover and motor-fault authority response are independent, unproven release gates; see w1ne/labwired#3190.
