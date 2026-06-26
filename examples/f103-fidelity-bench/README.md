# F103 fidelity benchmark

A faithful emulator must **fail the firmware that real hardware fails**. One that
passes a known-bad firmware gives a *false pass* — a green CI run hiding a bug.
This suite runs deliberately-broken firmware on LabWired and checks its verdict
against the STM32F103C8 datasheet, so false-pass prevention is measured, not
asserted. It doubles as a CI fidelity regression guard.

## Result

```
case       real-HW   LabWired
control    PASS      PASS
clockbug   FAIL      FAIL
gpiobug    FAIL      FAIL
rambug     FAIL      FAIL
                     4/4
```

LabWired reproduces the real silicon on every case because it models RCC clock
gating and the real 20 KB SRAM — the behaviour a passing test never exercises is
exactly where a false pass would otherwise hide.

## Cases

One firmware (`firmware/main.c`), one line changed each:

- `control` — correct; enables the USART1 clock → **PASS**
- `clockbug` — forgets `RCC_APB2ENR.USART1EN`; TXE never asserts → **FAIL**
- `gpiobug` — drives GPIOA without `IOPAEN`; writes dropped → **FAIL**
- `rambug` — stores 4 KB past the 20 KB SRAM; faults → **FAIL**

A case passes iff its marker (`BENCH_*_OK`) reaches the UART.

## Run

```bash
./run-benchmark.sh
```

Exits non-zero if LabWired ever disagrees with silicon, and writes
`benchmark-results.json`. `system-nogate.yaml` runs the same firmware on a
clock-gating-stripped chip: it false-passes there, showing the gates are what
catch the bug.

Needs `arm-none-eabi-gcc` and a built `labwired` (`cargo build -p labwired-cli`).
