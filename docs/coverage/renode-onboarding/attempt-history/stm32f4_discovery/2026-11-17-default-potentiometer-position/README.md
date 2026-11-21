# STM32F4-Discovery initial ADC attempt

The first bounded `--only stm32f4_discovery` run booted 4,000,000 instructions
but failed the new ADC assertion. With the potentiometer at its default 50%
position, UART reported `ADC1 IN1=000007FF EOC=1 FAIL`; the smoke expected the
25% input code `0x03FF`. This captured attempt is retained while the final smoke
sets the potentiometer to 25% explicitly.
