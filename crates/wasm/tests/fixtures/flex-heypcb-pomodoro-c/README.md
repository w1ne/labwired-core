# HeyPCB Pomodoro Pal: the original C source on Flex

This fixture compiles the firmware source file of the Pomodoro Pal, unchanged, to WebAssembly and runs it as portable Flex firmware. It is a functional simulation. It does not execute the original CH32V003 binary.

`flex-heypcb-pomodoro-timer` is a hand port of the same file to Rust. This fixture has no port: `firmware/pomodoro.c` is the pinned file, and its `int main(void)` with an endless loop is the entry.

## Source

- Project: [HeyPCB/pomodoro-pal at `1d6e50ff9998978e6f4521895042ae240fff7c42`](https://github.com/HeyPCB/pomodoro-pal/tree/1d6e50ff9998978e6f4521895042ae240fff7c42). Hardware license CERN-OHL-S-2.0. Firmware license MIT, Copyright (c) 2026 HeyPCB.
- `firmware/pomodoro.c` (git blob `0ece2a82608ad5925bbc9c03fa8efd83903af878`), `firmware/funconfig.h` (`df7a493c9944efec94555cbd1953f753aa0d0539`) and `firmware/LICENSE` (`fafa07eb7a4d778cb580b235b4457e2a1eefe999`) are byte for byte the pinned files. `git hash-object` gives these ids. `source-map.json` has their SHA-256, and the CLI test `the_pinned_c_firmware_source_is_unchanged_and_its_main_never_returns` compares them.
- `shim/` is new code, MIT. It is the part of the ch32fun API that `pomodoro.c` uses, written for Flex. It has no ch32fun code.
- `source-map.json` gives the net, MCU pad, series resistor and part of each port, from the pinned PCB blob `561268221ce4d066582156a71ffc5ec3e071f047`.

## How an endless `main` runs

A call step with `until_ns` does not need to return. The runner suspends the firmware inside the `advance_ns` that reaches that time, at exactly that time. A `set` step then changes the environment, and a `resume` step continues the same call to the next `until_ns`:

```yaml
sequence:
  - call: { export: __main_void, until_ns: 200000000 }
  - set: { port: touch_contact, value: 1 }
  - resume: { until_ns: 250000000 }
  - set: { port: touch_contact, value: 0 }
  - resume: { until_ns: 300000000 }
```

`__main_void` is the name that clang gives `int main(void)` on WebAssembly. The 15 `flex_bind_output` and `flex_bind_touch` steps before it are the board: each connects one MCU pin to one Flex port.

## What stands in for the CH32V003

| Original | This fixture |
|---|---|
| CH32V003 GPIO registers through ch32fun `funDigitalWrite` | A write to the Flex port of that pin, only when the level changes |
| SysTick counter, compare register and interrupt | A counter and compare value in memory at 48 counts for each microsecond. A compare match calls the firmware's own `SysTick_Handler`. |
| Soft-PWM phase from `SysTick->CNT >> 10` | The same expression on that counter |
| Code execution time | None. `Delay_Us` and `Delay_Ms` take the time they name. Each use of `SysTick` outside the handler takes 5,000 ns, so one idle or focus pass of the main loop is 5,000 ns. |
| Capacitive touch: ADC charge-time measurement in `ReadTouchPin` | 100 counts for each iteration with the Boolean `touch` port low, 200 counts with it high. The firmware's own baseline and threshold code runs on these counts. |
| Pin modes, the PD5 pull-up, clock gates | No effect |

The firmware's baseline, threshold, tap, hold, phase and LED code is the original code. `system.yaml` lists the assumptions and the unsupported checks; each `result.json` carries them in `run_report`.

## Scripts

Values that were calculated from the source before the first run, and that the first run then gave:

| Item | Value |
|---|---|
| First edge of `buzz` (four `SysTick` uses in `systick_init`) | 20,000 ns |
| Edges of `buzz` in the start-up chirp, 189,000 ns apart | 474, last at 89,417,000 ns |
| Edges of `min1`..`min11` after one tap | 1 |
| Edges of `min1` for tap, pause tap, 2.1 s hold | 6 |
| Edges of `buzz` for the hold reset | 474 more |
| End of an idle run with a 300 ms limit and no `until_ns` | 299,996,000 ns |

Values that the run measured: the ring lights at 269,026,000 ns (the first 20 ms touch poll after the release at 250 ms). The pause blink edges are at 600,026,000, 1,200,026,000, 1,800,026,000 and 2,400,026,000 ns. The reset chirp starts at 2,689,021,000 ns.

| Script | Virtual time | What it checks |
|---|---:|---|
| `test-tap-starts-focus.yaml` | 0.30 s | A 50 ms tap changes idle to focus. Minute LEDs 1..11 light once. The leaf LEDs and the piezo stay off after the start-up chirp. |
| `test-untouched-stays-idle.yaml` | 0.30 s | Negative control: with no tap, no minute LED after LED 0 lights. |
| `test-pause-and-hold-reset.yaml` | 3.00 s | A second tap pauses focus and the ring blinks. A 2.1 s hold goes back to idle with one chirp. The release after the hold is not a tap. |

All three pass on a dev build of the CLI: 13, 10 and 13 assertions. The 3.00 s script used 603,967,166 WebAssembly fuel units and 4.84 s wall time on the lane machine; the 0.30 s scripts used 0.5 s. A release build was not measured.

## Defect found by this fixture

`labwired test` on a dev build stopped with a host stack overflow on the first long firmware call. The CLI built wasmi without its `auto-dispatch` feature, so wasmi used its tail-call dispatch in a build profile that does not make tail calls. The CLI now enables `auto-dispatch`: a dev build uses the loop dispatch, a release build keeps the tail-call dispatch.

## Detailed target for this board (2026-11-19)

The project has the original binary, `firmware/pomodoro.bin` (git blob `dea79a189eb7a6ea8e526fa86b420da2f5f544e1`, SHA-256 `8877e57ccddf0ae8ef3b24dac0f913d9feebe57602fb8f307bb5de0fd4d458bd`). It was loaded at address 0 on `configs/chips/ch32v003f4p6.yaml` of this branch through `labwired test` with `inputs.system`. The run stopped after 746 cycles at PC `0x33e` with `memory_violation` at `0xe000f000`, the first SysTick write. That descriptor has RCC, GPIO and USART1 only. The detailed target cannot run this board's firmware on this branch. The binary is not in this fixture.

## Reproduce

```sh
CC="uv run --no-project --with ziglang==0.17.1 python -m ziglang cc" \
  sh crates/wasm/tests/fixtures/flex-heypcb-pomodoro-c/build.sh
cargo test -p labwired-cli --test flex_test_runner
cargo run -p labwired-cli --bin labwired -- test \
  --script crates/wasm/tests/fixtures/flex-heypcb-pomodoro-c/test-pause-and-hold-reset.yaml \
  --output-dir /tmp/pomodoro-c
```

`build.sh` also accepts clang with wasm-ld as `CC`. That build was not run here, and its `firmware.wasm` bytes can differ from the committed file.

## Not established

- The original `firmware/pomodoro.bin` does not run in this fixture. Instruction, GPIO, SysTick and ADC behavior of the CH32V003 are not simulated. This is not chip parity.
- Code execution time is not modeled. On hardware the main loop makes several passes in one 21.3 µs PWM phase step; here it makes about four, by assumption.
- The touch counts (100 and 200 for each iteration) are an assumption. The firmware README says the threshold is not tuned on hardware.
- The 25-minute focus phase and the break were not run with this fixture: at 5,000 ns for each pass they need about 360 million passes. `flex-heypcb-pomodoro-timer` has that run for the Rust port.
- J1, R1, R2, C1, C2 and the test points are not modeled. This is not full-board coverage.
- This fixture has no physical measurement and no browser run. `until_ns` and `resume` are a function of `labwired test`; the browser runtime does not have them.
- No comparable CH32V003 Rust-model onboarding time was measured. This fixture makes no speed claim against chip-specific model work.
