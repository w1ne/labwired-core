# nRF52840-DK smoke validation

The committed smoke builds `firmware-nrf52840-dk-smoke` for Cortex-M4 and runs
it on the focused `nrf52840-dk-led-smoke` system. Its GPIO edge assertion checks
the modeled physical LED1 path on P0.13 (Zephyr alias `led0`). The model
boundary is limited to that GPIO path; this is not a test of the radio, NFC,
USB, QSPI, or other board features.

The system has no modeled supply net. The simulator reports that it assumes an
ideal VDD rail, so power-on and brown-out behavior are outside this smoke.

Re-run the build and simulator smoke from the repository root with the command
in [`README.md`](README.md). Generated output is committed under `evidence/`.

The local onboarding smoke passes 2/2 assertions. The separate hosted
`labwired_verify` Arduino-template probe returned `failed` at 1,000,000 steps:
P0.13 had 0 transitions against a 16-edge oracle, and the runner remained at
PC `0x66e` before the sketch code. Its reported gaps were derived device time
(PLL reconfiguration is not tracked) and an assumed ideal power rail. The
hosted C build path is blocked because its Python 3.11 image requires Python
3.12; its Rust source path did not compile due to the missing Cortex-M interrupt
vector table. The next hosted-verifier step is to fix the nRF52840 startup path
and rerun the GPIO oracle. No hosted `model_verified` verdict or hardware
confirmation is claimed here.
