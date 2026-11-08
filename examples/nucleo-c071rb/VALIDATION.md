# NUCLEO-C071RB validation record

## Live vendor status checked 2026-11-03

- ST product page labels NUCLEO-C071RB **Active** and says the product is in volume production.
- ST's online store lists the board **In Stock** and provides an **Add to Cart** action; the recorded unit price was $19.51 at quantities 1–2.
- Evidence and source URLs are in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Exercised scope

| Path | Evidence |
|---|---|
| Cortex-M0+ reset-vector boot | `evidence/logs/run_smoke.log` |
| RCC_IOPENR GPIOA clock enable | `evidence/logs/run_smoke.log`, `evidence/simulation-output/result.json` |
| LD1 output on PA5 | `evidence/logs/run_smoke.log`, `evidence/simulation-output/result.json` |

The chip profile exposes only RCC_IOPENR and GPIOA. Its C0 RCC model is bounded
to the IOPENR register; the board button, USART2 VCP, USB, CAN-FD, and other MCU
peripherals are not in the profile. No zero-answering stubs are declared.

The hosted Part Knowledge catalog has no NUCLEO-C071RB record, so this is a
local repository-engine smoke. It is not a hosted `model_verified` result or
hardware confirmation. The simulator assumes an ideal supply rail, so
power-on and brown-out reset behavior is outside this smoke.
