# F103 fidelity benchmark

An emulator's job in CI is to **fail the firmware that real hardware fails**. One
that passes a known-bad firmware gives a *false pass* — a green run hiding a bug.
This benchmark runs the same firmware on LabWired and on Renode's own shipped
STM32F103 platform and scores each against the F103C8 datasheet.

## Result

```
case       real-HW   LabWired   Renode
control    PASS      PASS       PASS
clockbug   FAIL      FAIL       PASS <FALSE-PASS>
gpiobug    FAIL      FAIL       PASS <FALSE-PASS>
rambug     FAIL      FAIL       PASS <FALSE-PASS>
                     4/4        1/4
```

Renode false-passes the bug cases on its **own** `stm32f103.repl`: it has no RCC
clock-gating model (the clock-enable write logs as `unimplemented register
RCC:APB2ENR`) and maps a 256 MB SRAM instead of 20 KB. LabWired models both, so
it matches silicon.

## Cases

One firmware (`firmware/main.c`), one line changed each:

- `control` — correct; enables the USART1 clock → **PASS**
- `clockbug` — forgets `RCC_APB2ENR.USART1EN`; TXE never asserts → **FAIL**
- `gpiobug` — drives GPIOA without `IOPAEN`; writes dropped → **FAIL**
- `rambug` — stores 4 KB past the 20 KB SRAM; faults → **FAIL**

A case passes iff its marker (`BENCH_*_OK`) reaches the UART — the only signal
both engines expose, so they are judged identically.

## Run

```bash
./run-benchmark.sh                      # LabWired only
RENODE_BIN=/path/to/renode ./run-benchmark.sh   # add Renode
```

Exits non-zero if LabWired ever disagrees with silicon (use it as a CI fidelity
guard). Writes `benchmark-results.json`. `system-nogate.yaml` shows LabWired
false-passing too once the clock gates are stripped — the gates are what catch
the bug.

Needs `arm-none-eabi-gcc` and a built `labwired` (`cargo build -p labwired-cli`);
Renode optional.
