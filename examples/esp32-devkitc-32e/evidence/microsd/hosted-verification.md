# Hosted microSD model verification

`labwired_verify` returned `status: model_verified` for the smoke firmware on
the hosted classic ESP32 board. The SPI diagram attached `microsd` to GPIO18
(SCK), GPIO19 (MISO), GPIO23 (MOSI), and GPIO5 (CS), with 3V3/GND connected.
The oracle required one GPIO16 edge; the verifier observed exactly one edge
after CMD0, CMD8/R7, CMD55, and ACMD41 all returned their expected values.
It stopped with `stop_reason: assertions_passed`.

The hosted verifier reported one non-blocking approximation: VDD is an ideal
3.3 V rail because no power net drives the MCU, so power-on and brown-out reset
are not simulated. The repository-local smoke uses the exact DevKitC-32E
descriptor and system profile. This is digital-twin evidence, not a physical
board run. The card model does not implement a filesystem or persistent card
image.
