# Validation

## Supported slice

The system descriptor uses the existing ESP32-S3 chip model and connects its
UART0 console to a `host_console` endpoint representing the board's USB-to-UART
path; the bridge IC itself is not a separate model. The smoke runs the
committed Tier-1 image against this board system and checks UART0 output from
clock, GPIO, timer, interrupt, DMA, MCPWM, RMT and I2C firmware checks. The
committed run log reports 9/9 assertions passing; detailed logs and structured
result are in `evidence/run/`.

## Excluded board hardware

This profile is intentionally limited to the UART0 path. It does not claim
coverage for the v1.1 GPIO38 WS2812 RGB LED, PSRAM, USB-OTG, Wi-Fi/Bluetooth, or
the BOOT and RESET buttons. Those features need board-specific firmware and
acceptance evidence before they are included in the board profile.
