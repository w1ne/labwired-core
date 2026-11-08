# NUCLEO-G431RB

The [ST product page](https://www.st.com/en/evaluation-tools/nucleo-g431rb.html)
marks the board **Active / Product is in volume production**. The STM32G431RBT6
is also active and in stock, and an authorized distributor had the board in
stock at the live check recorded in
[`examples/nucleo-g431rb/evidence/vendor-status.md`](../../examples/nucleo-g431rb/evidence/vendor-status.md).

The smoke profile exercises the board's default STLINK-V3E VCP route through
LPUART1 on PA2/PA3 and the active-high LD2 user LED on PA5. Its committed
firmware prints `NUCLEO_G431RB_OK` and toggles PA5; the script asserts the UART
marker and at least eight GPIO edges.

The profile exposes RCC, GPIOA and LPUART1 only. It omits the button, the
remaining MCU peripherals, and USB behavior. No zero-answering stub is used by
the smoke profile. See
[`examples/nucleo-g431rb/README.md`](../../examples/nucleo-g431rb/README.md)
for reproduction steps, source links and the committed evidence.
