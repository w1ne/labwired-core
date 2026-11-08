# NUCLEO-F722ZE validation

The committed smoke builds `firmware-nucleo-f722ze-smoke` and runs it against
`configs/systems/nucleo-f722ze.yaml`. Passing requires at least 24 GPIO edges
on `gpiob:0` before the test's instruction limit.

The board profile is limited to RCC, GPIOB, PB0, and the active-high green LD1
signal, with SB120 ON and SB119 OFF for LD1's PB0 route. It does not declare
the board's ST-LINK, UART, Ethernet PHY, USB, user button, or other MCU
peripherals. No zero-answering peripheral stubs are used by this profile. This
is local simulator CLI evidence; no hosted `labwired_verify` verdict or
physical-hardware evidence was obtained, and the simulation does not model
board power sequencing.
