# Hosted verification — Raspberry Pi Pico PWM servo

`labwired_verify` returned `status: model_verified` for the explicit oracle:

- UART contains `PICO_PWM_SERVO_GP16_OK`.
- GP16 makes at least four transitions.

The successful run stopped after the assertions passed. The first 5.5-million-
step attempt ended in Arduino startup before `setup()`; the accepted retry used
the verifier's 50-million-step ceiling. The full request, source, result, retry
history and reported gaps are in [`hosted-verification.json`](hosted-verification.json).

The verifier reports two non-blocking approximations: it derives device time
from the board's declared 125 MHz `cpu_hz` without tracking PLL reconfiguration,
and assumes an ideal VDD rail because no power net was routed. The local smoke
separately checks the PWM DIV, TOP and CC_A register values. Neither result
claims measured physical timing, servo angle, electrical load or mechanical
movement.
