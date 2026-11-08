# Negative control: dual_usart with the CH32V003 AFIO layout

`chip.yaml` is `configs/chips/ch32v006f8u7-ch32fun-smoke.yaml` with one line
changed: the `afio` block's `profile: ch32v00x` becomes `profile: ch32v003`.
The CH32V003 AFIO keeps PCFR1 at 0x04, so ch32fun's store to the V00x PCFR1
at 0x0C (USART2_RM = 3) is dropped and USART2_RM reads 0.

Result (`run.log`): FAIL 4/5. `gpio_edges` on `gpiod:2` fails: PD2 never
moves, because the USART2_TX route is gated on USART2_RM = 3.
`check-dual-usart.log`: PD5 still decodes to the USART1 text, PD2 to nothing.

The merged UART console (`uart.log`) still shows the USART2 lines. The
console tap follows the USART's TX pad being an alternate-function output,
not the remap field, so the console text alone cannot tell the two runs
apart. That is why the smoke also asserts the pad edges, and why
`check-dual-usart.py` decodes the pads.

Run from a scratch copy of the tree (paths shown as `<negctl>`):
`labwired test --script <negctl>/examples/ch32v006f8u7/ch32fun-dual-usart-smoke.yaml`.
