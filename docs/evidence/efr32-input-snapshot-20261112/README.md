# EFR32MG26 input snapshot performance review

## Repository package

The review below records the state before this patch was applied. The exact patch and source excerpts are stored here as `efr32-input-snapshot.patch.gz` and `source-path-evidence.txt.gz`. Decompress them before checking their original hashes. The full GPIO source before and after the change is reproducible from the named base commit and patch; it is not duplicated in this package. `integration-receipt.json` records the payload hashes and packaging changes. Current test and performance results belong to [core PR #1852](https://github.com/w1ne/labwired-core/pull/1852).

The a0 CI failure is a deterministic instruction-count regression on one board/mode. The proposed one-file optimization has not yet been tested or measured; it leaves every performance threshold and fixture unchanged.

## Measured evidence

| Source | EFR32MG26 step Ir/step | Batch Ir/step |
|---|---:|---:|
| Last passing main, 102f7fc9 | 1509.7 | 2.2 |
| Existing gate baseline | 1513.7 | 2.2 |
| PR a0b0a91c | 1605.7 | 2.3 |

The measured change from passing main is exactly +96.0 Ir/step (+6.3589%). The gate comparison is +6.08%, above its unchanged 3% limit. Every other board's step result differs from that main run by at most 0.1 Ir/step. The batch increase passes the existing absolute floor; no rebaseline is proposed.

The failed sweep completed all 97 measured boards and 192 board/mode results, with 25 explicit waivers and no contract failures, skipped, stale or unmeasurable cases. Its single regression is EFR32MG26 step. The same workflow's real-time and real-workload checks passed. The board-throughput job exited 1 after finishing the sweep and uploading its report; it did not time out.

- Passing main run: https://github.com/w1ne/labwired-core/actions/runs/43287187879
- Failing exact-a0 run: https://github.com/w1ne/labwired-core/actions/runs/43306965051
- Failing board-throughput job: https://github.com/w1ne/labwired-core/actions/runs/43306965051/job/131493106283

The exact downloaded artifacts are retained as `passing-main-board-throughput-report.zip` and `failing-a0-board-throughput-report.zip`. `performance-comparison.json` records their hashes, member hashes, complete board/mode comparison, and source provenance. Main 102f to a0's parent 7778 changed only an example README. The performance workflow, harness, baseline file and spin fixture are unchanged between main and a0. The CI merge tree is the same tree as a0.

## Source path and proposed scope

The spin firmware performs no peripheral MMIO. The new motor write brackets are therefore not on its steady-state instruction path. The unchanged bus GPIO edge service polls all four EFR32 ports at each peripheral boundary. Each call reaches `GpioPort::read_gpio_input_word`, whose EFR branch previously fell through to `GpioPort::read_reg`.

The a0 change expanded that shared decoder with STM32-only alternate-function input checks after family decoding. EFR values remain unchanged, but its four input snapshots pass through the added decoder path. Four polls times 24 extra instructions matches the observed +96. This is strong static causal evidence; no retained per-function profile establishes how much is branch overhead versus changed compiler inlining.

The candidate adds one EFR32 arm beside the existing Nordic fast path. It calls the existing `effective_din()` only when Nordic offset translation is off and `window_offset <= 0x14`. In that case the address algebra resolves exactly to the EFR DIN register. Translated or saturated windows retain the unchanged decoder path, including unknown-register census. The same cached drive masks, pull resolution, external inputs and 16-bit mask produce the word. STM32 AF reads and all MMIO reads remain unchanged.

## Reviewable patch and controls

`efr32-input-snapshot.patch` is based on a148db88c953fdf926fcd92a09abb8289f14b9d1 and changes only `crates/core/src/peripherals/gpio.rs`. Full before/after source is saved next to the patch. The final patch SHA256 is 0144d677bbc4463327e54ee7ad2ee5ff9582e951f4d26ff72114235e99959557.

Two added tests cover 4,098 normal/translated window cases and mixed modes across all 16 pins. They check input pulls, push-pull, open-drain drive/release, external drive precedence, 16-bit masking, an installed peripheral route, invalid-window fallback, MMIO equality and single-pin equality. The new tests have not yet executed. Isolated rustfmt and `git apply --check` passed. No active checkout file was modified by this review.

`source-path-evidence.txt` retains the relevant committed source excerpts. No per-function Callgrind profile is present in either CI artifact. The harness writes its profile in a temporary directory and deletes it after extracting scalar counts.

Next validation belongs to the root's coordinated pipeline: apply the reviewed patch after the active native run, execute the focused GPIO/EXTI controls, and measure the same EFR32 step/batch contract with the normal release CLI. A focused pass does not replace the full unchanged CI gate. Keep profile output when measuring if attribution or a residual regression needs resolution.
