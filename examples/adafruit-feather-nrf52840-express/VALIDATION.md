# Adafruit Feather nRF52840 Express validation

This profile exercises the Feather D0/TX header through UARTE0 at 115200 baud.
The firmware configures P0.25, sends `FEATHER_NRF52840_UART_OK` using UARTE0
EasyDMA, and the local smoke asserts the UART marker.

Re-run the smoke with the command in `README.md`. The local logs and simulator
output are committed in `evidence/`. The separate hosted LabWired oracle result
is recorded in `evidence/labwired-verification.md`.

The model profile intentionally omits USB CDC, BLE/radio, the onboard LEDs and
NeoPixel, QSPI flash, battery charging, and all peripherals outside GPIO P0 and
UARTE0. The UART signal is exposed at Feather D0/TX; RX on D1/P0.24 is not
exercised. P1-based onboard LEDs are not claimed because the shared nRF52840
descriptor maps P1 through an engine-only address alias; this smoke stays on
P0.25, whose address matches the device map. Nordic documents the physical
P1 base at `0x50000300` in its [GPIO register map][gpio].

LabWired Part Knowledge returned `NOT_FOUND` for this exact board name. That is
a corpus gap, not evidence that the board lacks the mapped UART; the pin map
for this profile is taken from Adafruit's official variant source.

[gpio]: https://docs.nordicsemi.com/r/bundle/ps_nrf52840/page/gpio.html
