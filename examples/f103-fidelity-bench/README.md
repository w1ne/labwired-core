# F103 Fidelity Benchmark — false-pass prevention

A small, reproducible benchmark that measures whether an emulator **agrees with
real silicon** on firmware that is subtly broken. The point it makes: an
emulator's job in CI is not just to run firmware, it is to **fail the firmware
that real hardware would fail**. An emulator that passes a known-bad firmware
gives you a *false pass* — a green CI run that hides a bug that ships.

The same three firmware variants are run on LabWired and on
[Renode](https://renode.io) (using Renode's own shipped STM32F103 platform), and
each engine's verdict is compared against the STM32F103C8 datasheet ground truth.

## The cases

All three are built from one source (`firmware/main.c`), changing one line each.

| case | what it does | real-silicon verdict | why |
|------|--------------|----------------------|-----|
| `control`  | correct firmware: enables the USART1 clock, prints `BENCH_UART_OK` | **PASS** | positive control — proves the UART path and harness work |
| `clockbug` | identical, but forgets `RCC_APB2ENR.USART1EN` | **FAIL** | USART is clock-gated out of reset; `SR.TXE` never asserts, nothing transmits (RM0008 §7.3.7) |
| `rambug`   | enables the clock, then stores 4 KB past the end of the 20 KB SRAM | **FAIL** | `0x2000_6000` is unimplemented on an F103C8; the store faults (HardFault) before the marker prints |

A case **passes** iff its success marker (`BENCH_UART_OK` / `BENCH_RAM_OK`)
appears in the captured UART. That is the only signal both engines expose, so
they are judged identically.

## Result

```
case       real-HW      LabWired           Renode
----       -------      --------           ------
control    PASS         PASS               PASS
clockbug   FAIL         FAIL               PASS <FALSE-PASS>
rambug     FAIL         FAIL               PASS <FALSE-PASS>

fidelity score (verdicts matching real silicon):
  LabWired: 3/3
  Renode:   1/3
```

LabWired matches real silicon on every case. Renode false-passes both bug cases
— not because of a misconfiguration, but because its shipped `stm32f103.repl`:

- **has no RCC clock-gating model.** The clock-enable write is logged as
  `WriteDoubleWord ... to an unimplemented register RCC:APB2ENR`, and the USART
  transmits regardless of whether its clock was ever enabled.
- **maps a 256 MB SRAM** (`sram ... size: 0x10000000`) instead of 20 KB, so the
  out-of-bounds store silently succeeds.

This is consistent with Renode's documented design: its peripheral-authoring
guide instructs authors to *"not implement all registers — only those that are
actually used by the software."* That is a reasonable choice for bring-up, and a
dangerous one for regression CI, because the behaviour the firmware *didn't*
exercise is exactly where the false pass hides.

## The knob that catches the bug

LabWired's fidelity is what produces the correct fails — and it is opt-in. To
see LabWired behave like a low-fidelity emulator, run the `clockbug` firmware
against the clock-gating-stripped chip:

```bash
../../target/debug/labwired test --script clockbug-nogate-smoke.yaml
```

With the gates removed (`stm32f103-nogate.yaml`) the `clockbug` firmware
false-passes on LabWired too. The clock model is precisely what turns a false
pass into a real fail.

## Running it

```bash
# LabWired only (builds firmware, runs the three cases, scores against silicon):
./run-benchmark.sh

# Include Renode (uses its shipped stm32f103 platform):
RENODE_BIN=/path/to/renode ./run-benchmark.sh
```

The script exits non-zero if **LabWired** ever disagrees with the silicon ground
truth, so it doubles as a CI regression guard for LabWired's fidelity. Renode
mismatches are reported but do not fail the run — they are the measurement.

Requirements: `arm-none-eabi-gcc` to build the firmware; a built `labwired`
binary (`cargo build -p labwired-cli`); optionally a Renode launcher.

## Files

```
firmware/main.c              one source, three variants via -D flags
firmware/{startup.c,bench.ld,Makefile}
system.yaml                  real STM32F103 chip (20 KB RAM + clock gates)
system-nogate.yaml           same board, fidelity stripped (for the contrast)
stm32f103-nogate.yaml        derived chip with every `clock:` gate removed
control-smoke.yaml           LabWired test scripts, one per case
clockbug-smoke.yaml
rambug-smoke.yaml
clockbug-nogate-smoke.yaml   the false-pass-on-purpose demonstration
renode/run.resc.template     headless Renode run, parameterised by the runner
run-benchmark.sh             runs both engines and diffs against ground truth
```
