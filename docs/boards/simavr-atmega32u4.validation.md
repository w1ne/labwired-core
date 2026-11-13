# Imported ATmega32U4 onboarding smoke

The imported profile is limited to the AVR8 CPU, 32 KiB flash, 2.5 KiB SRAM,
and the GPIO port C window used by the fixture. The fixture sets DDRC7 as an
output, alternates PORTC7, and the smoke checks observed GPIOC7 transitions and
the configured instruction limit. USART, USB, timers, ADC, and other GPIO ports
are not part of this result.

The first attempt faulted during startup at `0xAFF`: the AVR core had selected
ATmega328P geometry for this imported name. Added distinct ATmega32U4 memory
geometry to the existing AVR8 execution path; the original failed run remains
in [`attempt-history`](../coverage/renode-onboarding/attempt-history/simavr-atmega32u4/initial-stack-boundary/).

The run assumes an ideal power rail; power-on and brown-out behavior are outside
this measurement. Evidence, including build/run logs, the result, JUnit
assertions, and snapshot, is under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega32u4/`](../coverage/renode-onboarding/metrics/simavr-atmega32u4/).

This is local LabWired CLI simulation evidence for the stated CPU/GPIO smoke. It
does not establish full ATmega32U4 coverage, Renode equivalence, or physical
hardware behavior.
