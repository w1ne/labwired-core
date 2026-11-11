# Validation

## UART profile

The system descriptor uses the existing ESP32-S3 chip model and connects its
UART0 console to a `host_console` endpoint representing the board's USB-to-UART
path; the bridge IC itself is not a separate model. The smoke runs the
committed Tier-1 image against this board system and checks UART0 output from
clock, GPIO, timer, interrupt, DMA, MCPWM, RMT and I2C firmware checks. The
committed run log reports 9/9 assertions passing; detailed logs and structured
result are in `evidence/run/`.

## v1.1 RGB profile

[`configs/systems/esp32s3-devkitc-1-v11-rgb.yaml`](../../configs/systems/esp32s3-devkitc-1-v11-rgb.yaml)
attaches the board's one-pixel WS2812 LED to the S3 GPIO model at GPIO38. The
firmware writes a red GRB frame to RMT0 memory, routes RMT0 through the GPIO
matrix, waits for TX_END, and holds the output low for the reset/latch interval.
The local CLI fixture is the PlatformIO ELF with its entry point redirected to
`labwired_entry()`. This harness skips Arduino/IDF startup, calls the same RMT
transmit routine used by `setup()`, and writes a UART completion marker. The
smoke checks that marker; a separate evidence checker requires the captured
device artifact to contain wire bytes `00 FF 00` (one red pixel in GRB order).
The committed merged flash image retains the ordinary Arduino app image.

The hosted `labwired_verify` run compiles the Arduino `setup()` source and its
serial oracle checks the post-TX_END marker. It returns `model_verified`; the
returned GPIO inspection artifact reports one decoded, lit WS2812 pixel. The
verification response is committed in `evidence/rgb/hosted-verify.json`. This
is model evidence for the serial oracle and decoded device output; it is not a
physical-board test. The verifier's power approximation assumes ideal VDD, so
power-on and brown-out behavior are not part of this result.

Evidence is committed under `evidence/rgb/`.

## Excluded board hardware

Both profiles are intentionally narrow. PSRAM, USB-OTG, Wi-Fi/Bluetooth, and
the BOOT and RESET buttons remain outside their claims. The RGB profile does
not model LED current draw or board power behavior.
