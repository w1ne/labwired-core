# HeyPCB Pomodoro Pal: the whole timer function on Flex

This fixture runs the application logic of the Pomodoro Pal firmware as portable Flex firmware, at the durations of the original: 25 minutes of focus, a three-note chirp, 5 minutes of break, a two-note chirp. It is a functional simulation. It does not execute the original CH32V003 binary.

## Source

- Project: [HeyPCB/pomodoro-pal at `1d6e50ff9998978e6f4521895042ae240fff7c42`](https://github.com/HeyPCB/pomodoro-pal/tree/1d6e50ff9998978e6f4521895042ae240fff7c42). Hardware license CERN-OHL-S-2.0. Firmware license MIT, Copyright (c) 2026 HeyPCB.
- `source-map.json` gives the git blob of each source file, and the net, MCU pad, series resistor and part of each port. The pad nets are read from the pinned PCB blob `561268221ce4d066582156a71ffc5ec3e071f047`.
- `firmware.rs` is a port of `firmware/pomodoro.c` (blob `0ece2a82608ad5925bbc9c03fa8efd83903af878`). The state machine, the durations, the ring and breathing rules, the tone timing and the tap and hold rules are those of the original.

## What is different from the original firmware

| Original | This fixture |
|---|---|
| CH32V003 GPIO registers through ch32fun | 14 Boolean write ports: `min0`..`min11`, `leaf`, `buzz` |
| SysTick interrupt counts milliseconds | Milliseconds are virtual time divided by 1,000,000 |
| Soft-PWM phase is `SysTick->CNT >> 10`, 1024 counts at 48 MHz (21,333.3 ns) | One `loop` call is one main-loop pass and takes 21,333 ns. The PWM period is 682,656 ns (1464.9 Hz). |
| The main loop runs many passes in one PWM phase step | One pass in one PWM phase step |
| Capacitive touch: ADC charge-time measurement, baseline that follows drift, threshold | One Boolean read port `touch`. The test bench sets `touch_contact`. |
| 16 baseline measurements before the start-up chirp | Not present |

The 14 LEDs and the piezo are logic-level followers of their MCU pin. LED current, the series resistors, light and sound are not modeled. `system.yaml` lists these assumptions and the unsupported checks; each `result.json` carries them in `run_report`.

## Scripts

| Script | Virtual time | What it checks |
|---|---:|---|
| `test-tap-starts-focus.yaml` | 0.30 s | A 50 ms tap changes idle to focus. Minute LEDs 1..11 light once. The leaf LEDs and the piezo stay off after the start-up chirp. |
| `test-untouched-stays-idle.yaml` | 0.30 s | Negative control: the same passes with no tap stay idle; no minute LED after LED 0 lights. |
| `test-pause-and-hold-reset.yaml` | 2.70 s | A second tap pauses focus. A 2.1 s hold goes back to idle with one chirp. The release after the hold is not a tap. |
| `long-focus-and-break.yaml` | 1801.55 s | One whole Pomodoro. See below. |

The CLI test `every_flex_fixture_runs_unchanged_through_the_generic_runner` runs each `test-*.yaml`, and fails each one on exactly one assertion when one expected call result is moved by one. It does not run `long-focus-and-break.yaml`.

## The whole Pomodoro (2026-11-19)

Values that were calculated from the firmware constants before the run, and that the run then gave exactly:

| Item | Value |
|---|---|
| Final virtual time | 1,801,552,783,656 ns |
| Start of the three-note chirp (end of focus) | 1,500,269,015,601 ns |
| Edges of `buzz` in that chirp (237 + 282 + 355 cycles) | 1,748 |
| Start of the two-note chirp (end of the break) | 1,800,269,003,706 ns |
| Edges of `buzz` in that chirp (237 + 282 cycles) | 1,038 |
| Edges of `buzz` between the chirps | 0 |
| Final call result | 16 (idle, 1 focus session done) |

Values that the run measured, and that the script now pins:

| Item | Value | Check |
|---|---|---|
| Edges of `leaf` in the break | 877,385 | At most 877,600: two for each PWM period in 299.55 s |
| Edges of `min11` in the last 125 s of focus | 365,989 | At most 366,216 |
| First edge of `leaf` | 1,500,717,737,601 ns | The first pass of the break |

The first run failed one assertion: it expected no `leaf` edge up to and including 1,500,717,737,601 ns. The firmware lights the leaf in the same pass that ends the chirp. The assertion was wrong by 1 ns and is corrected.

The committed script passes 16 of 16 assertions. `firmware.wasm` has SHA-256 `309c53063c814b4d7c467fb9c734c32e9652db0116c1053a324b3486cd8c0f79`; a rebuild from `firmware.rs` gave the same bytes.

Control for the long-run edge count: 12 s of the idle face records 71,244 edges. The CLI before this change gave `observed: null` for `digital_edges: { port: buzz, count: 474 }` on that run, because the capture had dropped 5,708 edges. The CLI with this change gives 474. The test `edge_assertions_stay_exact_after_the_logic_capture_overflows` holds this run.

Measured with the release CLI on the lane machine, one run with no other load: 364.68 s wall time for 1801.55 s of virtual time (4.9 times real time), 84,407,032 `loop` calls, 56,637,179,143 WebAssembly fuel units, 21 MB peak resident memory. The run recorded 13,183,032 logic edges. `flex-trace.json` holds the newest 65,536 and reports 13,117,496 as dropped.

## Reproduce

```sh
sh crates/wasm/tests/fixtures/flex-heypcb-pomodoro-timer/build.sh
cargo test -p labwired-cli --test flex_test_runner
cargo build --release -p labwired-cli --bin labwired
target/release/labwired test \
  --script crates/wasm/tests/fixtures/flex-heypcb-pomodoro-timer/long-focus-and-break.yaml \
  --output-dir /tmp/pomodoro-long
```

## Not established

- The original `firmware/pomodoro.hex` does not run in this fixture. Instruction, GPIO, SysTick and ADC behavior of the CH32V003 are not simulated. This is not chip parity.
- The touch threshold and baseline of the original are not exercised.
- J1, R1, R2, C1, C2 and the test points are not modeled. This is not full-board coverage.
- This fixture has no physical measurement.
- There is no browser run of this fixture. The edge windows and the exact long-run counts are a function of `labwired test`; the browser runtime has the 65,536-edge capture only.
- No comparable CH32V003 Rust-model onboarding time was measured. This fixture makes no speed claim against chip-specific model work.
