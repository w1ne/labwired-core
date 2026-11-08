# NUCLEO-H753ZI — LabWired smoke profile

The [NUCLEO-H753ZI](https://www.st.com/en/evaluation-tools/nucleo-h753zi.html)
board carries an STM32H753ZIT6. This package promotes a narrow runnable board
profile alongside the imported inventory descriptor. The ST eStore listed the
board as Active and In Stock with Add to Cart on 2026-11-04; the fitted MCU is
Active and in volume production. The MCU's separate eStore listing was Out of
Stock at the same check. See [the vendor status receipt](evidence/vendor-status.md).

## What runs here

The bare-register firmware in `crates/firmware-nucleo-h753zi-smoke`:

- leaves the HSI reset clock at 64 MHz,
- enables GPIOB/GPIOD/GPIOE and USART3,
- toggles the board's three active-high user LEDs: LD1 PB0, LD2 PE1, LD3 PB14,
- sends a banner through the modeled USART3 transmitter at 115200 baud. The
  pin map declares PD8/PD9 as the default ST-LINK VCP route; the ST-LINK USB
  bridge itself is not simulated.

The profile maps the 2 MiB flash and the 128 KiB DTCM used by this image. It
does not expose the other RAM banks or the board's Ethernet, USB, or other MCU
peripherals. The H7 RCC, GPIO, and USART blocks reuse the engine's existing
models; this H753 profile has not been checked against physical H753 silicon.
USART3 output is captured at the peripheral console; the GPIO alternate-
function connection from PD8 to the VCP is declared but not electrically
asserted by this smoke.

## Reproduce the smoke

From the repository root:

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-h753zi \
  --crate firmware-nucleo-h753zi-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-h753zi/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h753zi.yaml \
  --out-dir examples/nucleo-h753zi/evidence
```

The committed build log, smoke log, UART output, peripheral snapshot, and
onboarding metrics are under [`evidence/`](evidence/). The board LED and VCP
wiring follows ST's [UM2407 Rev 6](https://www.st.com/resource/en/user_manual/um2407-stm32h7-nucleo144-board-stmicroelectronics.pdf).
See [`VALIDATION.md`](VALIDATION.md) for the measured profile boundaries.
