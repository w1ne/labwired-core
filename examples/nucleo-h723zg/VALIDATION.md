# NUCLEO-H723ZG validation

Run `scripts/onboarding_smoke.sh` from the repository root using the command in
the README. It builds the firmware and CLI, then checks the committed smoke
against the narrow board profile.

The evidence directory records the build logs, simulator output, UART text,
GPIO edge assertions, generated metrics, and ST vendor-status check. No
physical NUCLEO-H723ZG was attached during this run.

The profile only claims RCC reset-clock access, GPIOB/GPIOD, USART3, DTCM, LD1
on PB0, and USART3 TX on PD8. Other H723 blocks and board peripherals are not
modeled in this profile.
