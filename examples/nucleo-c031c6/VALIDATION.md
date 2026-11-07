# NUCLEO-C031C6 validation record

## Live vendor status checked 2026-11-04

- ST marks NUCLEO-C031C6 **Active** and says it is in volume production.
- ST's online store showed the board **In Stock** with an **Add to Cart** action.
- The STM32C031C6 product page marks the MCU **Active**.
- Evidence and URLs are in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Exercised scope

| Path | Evidence |
|---|---|
| Cortex-M0+ reset-vector boot | `evidence/logs/run_smoke.log` |
| RCC_IOPENR GPIOA clock enable | `evidence/logs/run_smoke.log` and `evidence/simulation-output/result.json` |
| LD4 output on PA5 | `evidence/logs/run_smoke.log` and `evidence/simulation-output/result.json` |

The profile exposes only RCC_IOPENR and GPIOA. The button, VCP, USB, and other
MCU peripherals are not modeled by this board profile. The local repository
smoke is simulation evidence; it is not hosted `model_verified` or hardware
confirmation. The simulator assumes an ideal supply rail.
