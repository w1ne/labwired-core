# Validation scope: NUCLEO-H753ZI

The imported `nucleo_h753zi` system has a bounded local LabWired CLI smoke for
its Cortex-M7 execution path. This result covers the profile in
`configs/chips/onboarding/nucleo_h753zi.yaml`, not the full board or every
peripheral present on STM32H753ZI silicon.

The smoke executed 4,000,000 instructions and passed all 3 assertions:

- USART3 emitted the `STM32H7 register smoke` marker.
- GPIOB.0 produced an observed output edge.
- Execution stopped at the configured `max_steps` limit.

The profile declares a GreenLED attachment on GPIOB.0, but the smoke asserts
the GPIO edge only; it does not assert a rendered LED state.

The firmware enables the modeled H7 RCC gates, configures GPIOB/GPIOD and
USART3, and routes its UART output through PD8. The imported profile currently
maps 1 MiB of flash and 128 KiB of SRAM as bounded execution windows; these are
not a verified statement of the device's full memory density. Other board
peripherals are omitted from this profile and remain unverified. The run assumes
an ideal power rail, so power-on and brown-out behavior are outside its scope.

Logs, simulator result, JUnit assertions, and snapshot are committed under
`docs/coverage/renode-onboarding/metrics/nucleo_h753zi/`. This is local CLI
simulation evidence, not Renode differential validation or hardware
confirmation.
