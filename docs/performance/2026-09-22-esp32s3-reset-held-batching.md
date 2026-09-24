# ESP32-S3 reset-held batching

The original halted-secondary optimization widened a machine window without
preserving per-instruction peripheral and scheduler visibility. Real Arduino
firmware stopped booting, so that implementation was retired.

The replacement distinguishes three secondary-core states with one hot-path
query: active, WAITI-parked, and reset-held. While APP CPU is reset-held, PRO
CPU instructions share one host-side planning/commit window, but each retired
instruction still:

- publishes the live cycle;
- ticks peripherals;
- drains scheduler events;
- checks reset requests; and
- checks for APP CPU release, steps APP on that exact boundary, and ends the
  coalesced window immediately.

The loop uses `step_batch(..., 1)` and the returned retired count, so zero
progress neither consumes fuel nor charges simulated time. `CoreProgress`
explicitly records internally committed cycles, avoiding the double/lost cycle
accounting that invalidated the earlier generalized interval-one attempt.

## Callgrind A/B

Measured with `scripts/perf/board_perf.py` and the ESP32-S3 spin fixture:

| path | before (Ir/step) | after (Ir/step) | steps/batch |
| --- | ---: | ---: | ---: |
| ESP32-S3 batch | 815.7 | 364.8 | 1023.9 |
| ESP32-S3-Zero batch | 815.7 | 364.8 | 1023.9 |

That is 55.3% less host work per simulated instruction, or 2.24x throughput
at the same instruction mix. The reference step path remains separately gated.

## Correctness gates

- all `machine_advance` unit tests, including explicit reset-held
  zero-progress and elapsed-cycle cases;
- `e2e_esp32s3_flash_boot_no_elf` bare-IDF flash boot; and
- the same test's dual-core Arduino/FreeRTOS wide-batch boot, which detects the
  scheduler corruption caused by the retired implementation.
