# NUCLEO-G431KB

ST's current product page marks the board **Active / Product is in volume
production**; ST's eStore lists it **In Stock** with **Add to Cart**. The live
status receipt is committed at
[`examples/nucleo-g431kb/evidence/vendor-status.md`](../../examples/nucleo-g431kb/evidence/vendor-status.md).

The onboarding profile is intentionally limited to the board's default ST-LINK
VCP route: USART2 on PA2/PA3. Bare-metal firmware configures those pads, sends
`NUCLEO_G431KB_OK`, and is checked by the committed smoke script and simulator
log under `examples/nucleo-g431kb/evidence/`.

The profile exposes RCC, GPIOA and USART2, all modeled peripheral types. It
omits the user LED, board button, and other chip peripherals; no zero-answering
stub is declared in this profile. See
[`examples/nucleo-g431kb/README.md`](../../examples/nucleo-g431kb/README.md)
for reproduction steps and source references.
