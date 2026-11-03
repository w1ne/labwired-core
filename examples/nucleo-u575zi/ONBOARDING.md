# NUCLEO-U575ZI-Q onboarding

Checked and simulated on 2026-11-03. Current board lifecycle and store
availability are recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Board path covered

- Board system: [`configs/systems/nucleo-u575zi.yaml`](../../configs/systems/nucleo-u575zi.yaml)
- MCU model: [`configs/chips/stm32u575.yaml`](../../configs/chips/stm32u575.yaml)
- Firmware smoke: `firmware-stm32u575-demo`, built for
  `thumbv8m.main-none-eabi`; it writes `OK` to USART1 and the smoke asserts the
  UART marker and expected idle stop.
- Board wiring: ST-LINK VCP is USART1 on PA9/PA10 (AF7), captured through
  `debug_uart: usart1`. LD1 is PC7, LD2 PB7, LD3 PG2, and B1 PC13 in the system
  descriptor. This smoke asserts only USART1 output; it does not claim LED or
  button behavior.

Reproduce the committed build and simulator output with:

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32u575-nucleo \
  --crate firmware-stm32u575-demo \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-u575zi/io-smoke.yaml \
  --system configs/systems/nucleo-u575zi.yaml \
  --out-dir examples/nucleo-u575zi/evidence
```

The captured run passed both smoke assertions. Build logs, simulator output,
UART bytes, and the generated scoreboard are committed in [`evidence/`](evidence/).

## Model boundary

The board's USART1 VCP path is backed by the native STM32U575 UART model. The
chip descriptor installs no zero-answering stub on this smoke path and leaves
unmodeled blocks undeclared. In particular, TrustZone/GTZC/SAU enforcement,
OCTOSPI/HSPI, USB OTG, Ethernet, and ADC4 are not modeled. USART RX is not
pad-routed, so this test proves captured UART output only, not bidirectional
USB serial. The wider chip model is SVD/RM0456-derived and has no silicon
bench comparison; see [`docs/boards/stm32u575.md`](../../docs/boards/stm32u575.md)
for the full capability and fidelity limits.

The hosted LabWired verifier currently supports Arduino on this board profile,
not the repository's Rust smoke. Its separate Arduino `Serial1` check is saved
in `evidence/mcp-oracle-check.ino` and `evidence/mcp-oracle-check.json`; it
returned `status: model_verified` for the VCP marker. That result assumes an
ideal power rail and does not verify power-on or brown-out behavior. It is
separate from the committed Rust smoke and is not hardware evidence.
