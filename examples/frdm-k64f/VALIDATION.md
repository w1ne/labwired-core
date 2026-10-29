# Validation — FRDM-K64F (MK64FN1M0VLL12)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x400, SP: 0x20030000` |
| Cortex-M4 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| SIM clock gates (`SCGC4` UART0, `SCGC5` PORTB/PORTE) | declared only | the SIM window is a stub; the firmware writes the gates for silicon realism, the simulator does not gate on them |
| GPIOB/GPIOE output drive + PDOR readback | smoke-tested | `led=red on pin=0` … asserted in `io-smoke.yaml` |
| UART0 PTB16/PTB17 TX @ 115200 from 20.97152 MHz FEI | smoke-tested | `FRDM-K64F SMOKE` + heartbeat lines on the console sink |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| PORT pad mux/pulls | **not modelled** | PORTx PCR is a stub |
| MCG PLL/FEE | **not modelled** | the smoke uses the reset FEI clock |
| ENET + KSZ8081 PHY / TCP | **not modelled** | the Ethernet blocks are stubs; the Zephyr TCP test is deferred |
| RNGA, FTFE, eDMA, PIT/FTM, I2C/SPI, ADC/DAC, USB, CAN, SDHC | declared stubs | `configs/chips/mk64f12.yaml` |

## Commands and evidence (verified 2026-10-29)

```bash
# from the repository root
cargo build -p firmware-frdm-k64f-smoke --release --target thumbv7em-none-eabi
#   Finished `release` profile — no warnings

./target/release/labwired test \
  --script examples/frdm-k64f/io-smoke.yaml \
  --output-dir out/frdm-k64f-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.01s

./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-frdm-k64f-smoke \
  --system configs/systems/frdm-k64f.yaml --max-steps 400000
#   Initial PC: 0x400, SP: 0x20030000
#   FRDM-K64F SMOKE
#   cpu=cortex-m4 uart=uart0
#   led=red on pin=0
#   ...
#   hb=0 red=0 green=1 blue=1

# Determinism: two runs, identical UART + snapshot
cmp out/run1/uart.log out/run2/uart.log          # clean
cmp out/run1/snapshot.json out/run2/snapshot.json  # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-frdm-k64f-smoke \
  --system configs/systems/frdm-k64f.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/frdm-k64f
#   Instructions executed: 200000 · Unsupported: 0
```

## Notes

- The heartbeat reports the actual `PDOR` bit for each LED (0 = lit, because
  the LEDs are active low), so a passing run proves the pad the firmware drove.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The K64F `core` is `cortex-m4` and the firmware is built for
  `thumbv7em-none-eabi` (integer ABI); the M4F FPU is not exercised.
- The board's UART0 pins are PTB16 (RX) / PTB17 (TX) per the Zephyr pinctrl
  file. The survey text listed them in the opposite order; the descriptor and
  the firmware follow the board dts.
