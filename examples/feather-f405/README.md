# Adafruit Feather STM32F405 Express

This board profile boots firmware on the active STM32F405RG and exercises the
Feather's red D13 LED (PC1) and its exposed USART3 TX pin (D1/TX, PB10). The
smoke configures USART3 at 115200 baud, toggles D13, reads its output state
back, and sends the observed state over the UART.

## Run the smoke

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

The run builds the firmware and CLI, then checks the UART marker and ordered
LED-state heartbeat. The committed build, run, and simulation artifacts live
in [`evidence/`](evidence/); the current vendor check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
The command clears `RUSTC_WRAPPER` because this workspace's configured
`sccache` executable is missing.

## Model boundary

The board system exposes the D13 red LED on PC1 and the USART3 TX/RX header
pins PB10/PB11. The descriptor has no PC13 user button: the board guide and
schematic show no such button. The smoke's UART output is the on-board USART3
pin, not the USB-C connector's USB serial path.

The board also includes a NeoPixel, 2 MB SPI flash, microSD/SDIO, USB, and
battery charging. Those board-level devices and paths are not modeled by this
profile. The MCU descriptor contains additional peripheral instances, but
this smoke claims only the exercised GPIO and USART3 behavior. No physical
Feather was attached for this simulator run.
