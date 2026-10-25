# Validation — STM32U575I-EV

Claims, evidence, and what is explicitly **not** claimed.

Tier: **L1 smoke, SIM-DERIVED**. There is no U575I-EV bench part, no silicon
capture and no SWD/MMIO differential. The 60North `renode-stm32u5` package is a
**Renode-derived reference, not silicon** (see
[`../../validation/stm32u5-60north/README.md`](../../validation/stm32u5-60north/README.md)).

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from the committed ELF (reset vector, PC in flash) | smoke-tested | io-smoke run below |
| Cortex-M33 executing the committed firmware | smoke-tested | `labwired test`: 3/3 checks, 4,000,000 steps |
| RCC AHB2ENR1 GPIOB/GPIOH gate + APB2ENR USART1 gate | smoke-tested | LEDs and console only respond after the firmware enables the clocks |
| USART1 console (COM1 PA9/PA10) | smoke-tested | banner + heartbeat on the TX sink |
| LED5 PB7 / LED6 PH7 independent drive | smoke-tested | `odr_b=`/`odr_h=` read back after each write (`0x80`/`0x00`) |
| Determinism (same inputs → same output) | smoke-tested | two runs byte-identical (`diff` clean) |
| PR-gate survival (`firmware_survival`) | smoke-tested | `test_stm32u575i_ev_smoke_survival` |
| Instruction-support audit | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| LED7/LED8 (MFX I/O expander), LCD, camera, Octo-SPI, SD, audio, EEPROM | **not modelled** | see `EXTERNAL_COMPONENTS.md` |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED |

## Commands and evidence (2026-10-25)

```bash
rustup target add thumbv8m.main-none-eabi   # once
cargo build -p firmware-stm32u575i-ev-smoke --release --target thumbv8m.main-none-eabi
cp target/thumbv8m.main-none-eabi/release/firmware-stm32u575i-ev-smoke \
   tests/fixtures/stm32u575i-ev-smoke.elf
# sha256 ac3ab0a7d325026ba296efd01d9d842dffec7fc4aab3d256923c0b6578e67782

labwired test --script examples/stm32u575i-ev/io-smoke.yaml
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.02s
```

Captured console prefix:

```text
STM32U575I-EV SMOKE
cpu=cortex-m33 uart=usart1
leds=off odr_b=00000000 odr_h=00000000
led5=on odr_b=00000080 odr_h=00000000
led6=on odr_b=00000080 odr_h=00000080
led5=off odr_b=00000000 odr_h=00000080
led6=off odr_b=00000000 odr_h=00000000
hb=0 odr_b=00000080 odr_h=00000080
...
```

The `odr_*` values are read back from GPIOB_ODR / GPIOH_ODR after the write, so
the assertion checks the pad the firmware actually drove.

### Determinism

```bash
labwired --firmware tests/fixtures/stm32u575i-ev-smoke.elf \
  --system configs/systems/stm32u575i-ev.yaml --max-steps 4000000 > run1.txt
labwired --firmware tests/fixtures/stm32u575i-ev-smoke.elf \
  --system configs/systems/stm32u575i-ev.yaml --max-steps 4000000 > run2.txt
diff run1.txt run2.txt && echo DETERMINISTIC
#   DETERMINISTIC
```

### PR-gate survival

```bash
cargo test -p labwired-core --test firmware_survival test_stm32u575i_ev_smoke_survival
#   test test_stm32u575i_ev_smoke_survival ... ok
```

### Unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/stm32u575i-ev-smoke.elf \
  --system configs/systems/stm32u575i-ev.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/stm32u575i-ev
#   unknown_thumb16: 0
#   unhandled_thumb32: 0
#   unknown_riscv: 0
#   unsupported_total: 0
```

## Non-claims

- **No silicon parity.** The board page and this runbook never claim a bench
  capture. Both the LabWired model and 60North are software models.
- **No U545 differential.** The 60North suite targets the U575; the U545 is a
  separate derivative.
- The STM32U575I-EV's expander-driven LEDs, display, camera, external memory
  and audio are out of scope for this smoke.
