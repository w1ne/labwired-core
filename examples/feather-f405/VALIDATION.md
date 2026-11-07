# Adafruit Feather STM32F405 Express validation

## What the smoke checks

The board uses D13 / PC1 for its red LED and USART3 on PB10/PB11 for its TX/RX
header pins. The firmware enables the GPIOB/GPIOC and USART3 clocks, configures
PB10 for USART3 AF7, configures PC1 as output, and repeatedly toggles the LED.
It reads back GPIOC ODR before reporting each state over USART3. The smoke
asserts the ready marker and an ordered sequence of on/off states.

The USART3 instance was added to the STM32F405 descriptor with its RM0090 base
address, IRQ, and APB1 clock gate. USART3 uses the engine's existing STM32F4
UART model; the chip's USART3 register description records the F4 SR/DR layout.

## Reproduce and evidence

From the repository root:

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id feather-f405 \
  --crate firmware-feather-f405-demo \
  --target thumbv7em-none-eabi \
  --script examples/feather-f405/io-smoke.yaml \
  --system configs/systems/feather-f405.yaml \
  --out-dir examples/feather-f405/evidence
```

The committed `evidence/logs/` and `evidence/simulation-output/` files are the
build and deterministic simulator record. This is simulator evidence only;
no physical board was connected. The Core Onboarding Smoke workflow runs this
same CLI smoke on main pushes, scheduled runs, and manual dispatch. The hosted
LabWired catalog currently lists `feather-f405` as `compilable: false`, and
the verifier's accepted board list does not include this target, so it could
not issue a hosted `model_verified` verdict. This record claims only the
local CLI smoke result.

## Deliberate limits

The smoke covers the red D13 LED and USART3 TX. It does not validate the
NeoPixel, 2 MB SPI flash, microSD/SDIO slot, USB-C data/DFU path, battery
charger, or unrelated F405 peripherals. PC13 is not an on-board user button
on this Feather and is not exposed as board I/O.
