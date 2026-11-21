# Fork correctness merge-back — 2026-11-21

## Purpose

Current upstream retained several register, interrupt, event-ordering and duration-conversion defects corrected in the external CrispStrobe fork. This change ports the narrow corrections and their regression tests onto upstream `2388691da930913b9d265d4bf9f5e71526ef7106`.

The source review used fork main `aa0ec2bf8685e37c1848df4f355696eb2c0e2e12` and the newer scheduler-clock correction on `merge/upstream-20261120-full`. Changes to broad execution accounting, fork-specific C3 optimization, PWM playback, RGB/touch, DMAC request latency and DFLL tolerance are outside this batch.

## Source attribution

The selected MIT-licensed source commits were authored by **crispasr integration** in [CrispStrobe/labwired-core](https://github.com/CrispStrobe/labwired-core). The integration commit preserves co-authorship. Production hunks were applied to current upstream files; entire older fork files were not substituted.

| Port | Source commit |
|---|---|
| Nordic GPIO aliases and narrow stores | [`db327c08cab1`](https://github.com/CrispStrobe/labwired-core/commit/db327c08cab1b60e95a24cc425c78c216d325aef) |
| SAADC channel limits and IRQs | [`8220a7030fc4`](https://github.com/CrispStrobe/labwired-core/commit/8220a7030fc49e434def34f1c9b525cda097a168) |
| SAADC reset resolution | [`47dd99f3d0bd`](https://github.com/CrispStrobe/labwired-core/commit/47dd99f3d0bd0aa99fa1ac60df31565ac4a699d5) |
| EGU latched-event IRQ | [`39187d1a7833`](https://github.com/CrispStrobe/labwired-core/commit/39187d1a7833b48f8fd6b641e211a4c0d0da189c) |
| TIMER simultaneous compare shortcuts | [`c8b6081f6bdf`](https://github.com/CrispStrobe/labwired-core/commit/c8b6081f6bdfc3f998ff7f1a2b332d4b4949eaa5) |
| RTC prescaler restart | [`8515c4c3bb18`](https://github.com/CrispStrobe/labwired-core/commit/8515c4c3bb187c8caaef3c43e9af4c76601434c6) |
| WDT reset defaults | [`86039d95b399`](https://github.com/CrispStrobe/labwired-core/commit/86039d95b39945a49bbef193e685bdf384b043b2) |
| TEMP result and completion latch | [`0fd7be25b85f`](https://github.com/CrispStrobe/labwired-core/commit/0fd7be25b85f264d8672ae2161eb08cb4affe9e1) |
| ESP32-S3 USB enumeration speed | [`5b9aef92a7d3`](https://github.com/CrispStrobe/labwired-core/commit/5b9aef92a7d3d341f6a49472cdb85d67fdb29a2b) |
| SAMD SPI live clock timing | [`d94c64289253`](https://github.com/CrispStrobe/labwired-core/commit/d94c64289253886092e5657e82358acf58343194) |
| Cortex-M cached event deadline | [`84d34eba035a`](https://github.com/CrispStrobe/labwired-core/commit/84d34eba035a8f6b7b2c4560daceb8cbb74dac13) |
| Custom Bus flash-operation boundary | [`064dc750c28d`](https://github.com/CrispStrobe/labwired-core/commit/064dc750c28d687436d3e77235d5d839668d471d) |
| Publish scheduler time without an empty drain | [`fb6797db9737`](https://github.com/CrispStrobe/labwired-core/commit/fb6797db97377ce1a82b6e99b7117e0cfdc1621d) |
| Session zero and bounded duration arithmetic | [`e068b64e878a`](https://github.com/CrispStrobe/labwired-core/commit/e068b64e878a45c38037516a3fb50ca205d1b0f8) |
| Exact Python nanosecond ceilings | [`f8ebdcde2bf0`](https://github.com/CrispStrobe/labwired-core/commit/f8ebdcde2bf0a28a5c1e1aa338717fa24c3e6f54) |
| Bound RISC-V run chunks before narrowing | [`1e1aeb5a0999`](https://github.com/CrispStrobe/labwired-core/commit/1e1aeb5a0999cf26ab5a084eeefc07aefe93dbac) |
| Report early benchmark retirement | [`5d729bd30cae`](https://github.com/CrispStrobe/labwired-core/commit/5d729bd30cae857d030a8b2f6f24f707d96ca784) |

## Integration decisions

- Preserve current upstream IRQ reconciliation, PWM playback and execution accounting.
- Take the SAADC limit-event and reset-resolution corrections together.
- Move source integration tests into the current grouped SAMD and session targets.
- Use the corrected countdown-test fixtures from fork main, including follow-up commits `996e296d139b`, `995a944fd45b`, `0f0f5eb141a9` and `d7421d16cbcc`.
- Extract only the bounded run-chunk calculation and its regression test from the broader CLI accounting commit.
- Exclude the USB commit's unrelated hosted-window test, which uses fork-only execution APIs.
- Preserve performance tolerances and captured-hardware provenance. This change does not create physical capture evidence.

## Deferred idle-skip change

The timer-only skip removal from [`ed6b91377553`](https://github.com/CrispStrobe/labwired-core/commit/ed6b9137755304f5267566059d3f06b384f3ed58) is deferred. Current RISC-V has no polling-proof implementation to replace the heuristic, and this port fails the existing C3 SYSTIMER/GPIO acceleration regression. Its broader fork counterpart spans multiple execution and frontend files. A compatible fix needs bounded loop recognition and differential checks of registers, PC, timer latch state, GPIO changes, and event deadlines. This batch retains the existing upstream test and behavior; it does not claim the bus-only heuristic is proven safe.

## Validation

The regressions cover reset values, latched interrupts, byte and halfword stores,
DMA-limit boundaries, simultaneous compare shortcuts, event-delivery order,
live peripheral and CPU clock changes,
and exact duration arithmetic. The SAMD clock tests exercise both forced peripheral
walking and scheduler delivery. The session no-op regression requires a built
firmware fixture when `LABWIRED_REQUIRE_FIRMWARE=firmware-ci-fixture` is set.

Validation commands used for this integration are:

```sh
cargo test -p labwired-core --lib
cargo test -p labwired-core --features event-scheduler --lib
cargo test -p labwired-core --test samd samd51_sercom_timing
cargo build --release -p firmware-ci-fixture --target thumbv6m-none-eabi
LABWIRED_REQUIRE_FIRMWARE=firmware-ci-fixture cargo test -p labwired-core --test session session_uart
cargo test -p labwired-cli --lib run_chunk_preserves_u64_budget_and_refuses_overrun
maturin develop --manifest-path crates/python/Cargo.toml
python -m pytest crates/python/tests/test_duration.py -q
python -m pytest scripts/perf/test_board_perf.py -q
```

The normal pre-commit and pre-push hooks apply. The PR carries the `perf` label
because execution-loop behavior changes; the board-throughput workflow and
protected branch checks determine merge readiness. Completed results are recorded
in the PR so this attribution record does not imply that an unrun check passed.
