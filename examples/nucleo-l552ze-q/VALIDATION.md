# Validation — NUCLEO-L552ZE-Q (STM32L552ZET6Q)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x8000800, SP: 0x20030000` |
| Cortex-M33 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| L5 RCC clock gates (`AHB2ENR` GPIO, `APB1ENR1.PWREN`, `APB1ENR2.LPUART1EN`) | smoke-tested | firmware bring-up + LPUART output |
| VDDIO2 enable (`PWR_CR2.IOSV`) before PG7/PG8 | written by the firmware, **not enforced** | the firmware sets IOSV as silicon requires; see the VDDIO2 row below |
| GPIO PC7/PB7/PA9 drive + ODR readback | smoke-tested | `led=ld1 odr=00000080` … asserted in `io-smoke.yaml` |
| LPUART1 PG7 TX @ 115200 from 4 MHz MSI | smoke-tested | `NUCLEO-L552ZE-Q SMOKE` + heartbeat lines on the console sink |
| PC13 button input | smoke-tested (reads 0 released) | `btn=0` line |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| Stock Zephyr v3.7.0 `samples/hello_world` (`nucleo_l552ze_q`) | smoke-tested | `zephyr-hello.yaml` 2/2 (`*** Booting Zephyr OS build v3.7.0 ***` then `Hello World! nucleo_l552ze_q/stm32l552xx`); `firmware_survival::test_stm32l552_zephyr_survival` |
| ICACHE (`0x4003_0400`) | modelled — the U5 `icache` model, same IP (RM0438 §8 = RM0456 §8) | Zephyr's soc init polls `SR.BUSYF` and sets `CR.EN`; unmapped, it bus-faulted before `main` |
| DBGMCU (`0xE004_4000`) | modelled — `IDCODE 0x2001_6472` (DEV_ID 0x472, rev Z; not silicon-captured) | Zephyr clears `DBG_STOP`/`DBG_STANDBY`; unmapped, it bus-faulted |
| FLASH `ACR.LATENCY` read-back | **not modelled** — fidelity gap | the FLASH controller is a stub, so `LL_SetFlashLatency` reads 0 back, gives up after two reads and returns `ERROR`, which Zephyr ignores. Silicon latches 5 wait states for 110 MHz |
| TrustZone / SAU / GTZC / secure aliasing | **not modelled** | core stores MPU registers but does not enforce them |
| MPU / PMSAv8 / `TT`/`TTA` | **not modelled** | `docs/isa_coverage.md` known gap |
| VDDIO2 isolation of PG[15:2] until `PWR_CR2.IOSV` | **not modelled** — fidelity gap | the sim drives port G whether or not IOSV is set, so a firmware that forgets IOSV passes here and stays silent on the board. The smoke sets IOSV (RM0438 PWR chapter; CMSIS `PWR_CR2_IOSV_Pos = 9`) |
| L5 non-secure FLASH controller, EXTI, ADC, FDCAN, USB | declared stubs | `configs/chips/stm32l552.yaml` |
| SPI/I2C/timers/DMA/RNG/RTC | modelled, **unexercised** | declared; no firmware path in this slice |
| USB / Ethernet / radio | **not modelled** | — |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-l552ze-smoke --release --target thumbv8m.main-none-eabi
#   Finished `release` profile — no warnings

./target/release/labwired test \
  --script examples/nucleo-l552ze-q/io-smoke.yaml \
  --output-dir out/nucleo-l552ze-q-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.81s

./target/release/labwired \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-l552ze-smoke \
  --system configs/systems/nucleo-l552ze-q.yaml --max-steps 400000
#   Initial PC: 0x8000800, SP: 0x20030000
#   NUCLEO-L552ZE-Q SMOKE
#   cpu=cortex-m33 uart=lpuart1
#   led=ld1 odr=00000080
#   ...
#   hb=0 odr=00000080 btn=0

# Determinism: two runs, identical UART + snapshot
./target/release/labwired test --script ... --output-dir out/run1 --no-uart-stdout
./target/release/labwired test --script ... --output-dir out/run2 --no-uart-stdout
cmp out/run1/uart.log out/run2/uart.log        # clean
cmp out/run1/snapshot.json out/run2/snapshot.json  # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-l552ze-smoke \
  --system configs/systems/nucleo-l552ze-q.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/nucleo-l552ze-q
#   Instructions executed: 200000 · Unsupported: 0
```

### Stock Zephyr hello_world (verified 2026-10-30)

The fixture is unmodified upstream Zephyr v3.7.0 (commit `36940db938a8`)
`samples/hello_world` for `nucleo_l552ze_q`, built with
`arm-none-eabi-gcc` 13.2.1 (gnuarmemb) and debug-stripped. It is the sample
Renode's `ARMv8M.robot` "Should Boot Zephyr Hello World" runs. Renode loads
the `_ns` build behind TF-M. LabWired does not model TrustZone, so it boots
the TrustZone-disabled board target, which runs secure from `0x0800_0000`.

```bash
crates/firmware-l552ze-zephyr/build.sh      # needs a Zephyr v3.7.0 west workspace
#   sha256 f590906969f282b0642499de991245239cf45aafe34f09343b711bdb22cdc219
#   (two rebuilds, identical)

./target/release/labwired test --script examples/nucleo-l552ze-q/zephyr-hello.yaml
#   *** Booting Zephyr OS build v3.7.0 ***
#   Hello World! nucleo_l552ze_q/stm32l552xx
#   PASS  2/2 checks · zephyr-hello · 2000000 steps

# Determinism: two runs with --output-dir; uart.log and snapshot.json cmp clean

cargo test -p labwired-core --test firmware_survival l552
#   test_nucleo_l552ze_q_smoke_survival ... ok
#   test_stm32l552_zephyr_survival ... ok

./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/stm32l552-zephyr-hello.elf \
  --system configs/systems/nucleo-l552ze-q.yaml --max-steps 2000000 ...
#   instructions_executed 1999998 · unsupported_total 0
```

### Stock Zephyr synchronization (verified 2026-10-31)

Unmodified upstream Zephyr v3.7.0 `samples/synchronization` for the same
board target, built by the same script (which pins the Zephyr module list, so
the hello fixture above still rebuilds to `f5909069…`). Two threads hand a
semaphore back and forth; each prints, busy-waits 100 ms (`k_busy_wait`,
11 M cycles at Zephyr's 110 MHz clock) and then sleeps 500 ms on the SysTick
system timer. The `thread_b` line needs the SysTick timeout wake, the
semaphore give/take and a PendSV context switch; the second `thread_a` line
needs a full a -> b -> a round. No model change was needed.

```bash
crates/firmware-l552ze-zephyr/build.sh
#   sha256 45229cd392a0b1378145022519ed7db22e58fb2e104f034739f57d9bb353f0de
#   (two rebuilds, identical)

./target/release/labwired test --script examples/nucleo-l552ze-q/zephyr-sync.yaml
#   *** Booting Zephyr OS build v3.7.0 ***
#   thread_a: Hello World from cpu 0 on nucleo_l552ze_q!
#   thread_b: Hello World from cpu 0 on nucleo_l552ze_q!
#   thread_a: Hello World from cpu 0 on nucleo_l552ze_q!
#   PASS  2/2 checks · zephyr-sync · 30000000 steps

# Determinism: two runs with --output-dir; uart.log and snapshot.json cmp clean
# Negative control: max_steps 8000000 stops before the timeout round -> FAIL 1/2

./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/stm32l552-zephyr-sync.elf \
  --system configs/systems/nucleo-l552ze-q.yaml --max-steps 30000000 ...
#   unsupported_total 0
```

Path exercised by the unpatched Zephyr bring-up: ICACHE enable, DBGMCU,
the L5 RCC MSI range 6 (4 MHz) -> PLL (`M=1 N=55 R=2`, 110 MHz) switch with
the `SWS` handshake, `PWR_CR2.IOSV` (VDDIO2, written by Zephyr's GPIO
driver for port G) and the LPUART1 console on PG7/PG8.

## Notes

- The heartbeat's `odr=` field is read back from the port's `ODR` after each
  write, so the assertions check the pad the firmware actually drove.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The `stm32l5` RCC profile is reference-manual/CMSIS derived
  (`stm32l552xx.h`), not silicon-verified: there is no L552 bench part.
- The board's real LEDs are LD1 PC7, LD2 PB7, LD3 PA9 (UM2581 and the Zephyr
  board dts). Some test platform fixtures wire a stand-in green LED to PA5
  (Arduino D13); that is not this board's LD1.
