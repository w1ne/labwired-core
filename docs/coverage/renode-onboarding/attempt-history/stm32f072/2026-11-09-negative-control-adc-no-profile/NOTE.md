# Negative control: the ADC without `profile: stm32f0`

Same firmware, system and scenario as the passing run (summary
0x073FFFFF), with one change to the chip profile: the `adc` entry has no
`profile`, so the factory falls back to the default `adc.rs` layout (the
SQRx-sequencer STM32 ADC) instead of the F0/L0 CHSELR model.

Result: FAIL 6/8 assertions (uart_ordered and memory_value). Everything up
to and including the CRC line passes; the firmware never prints its `adc`
line or its summary within the 4M-step budget (stop reason max_steps). The
default layout has no CHSELR/ISR at the F0 offsets, so the calibration,
ADRDY and EOC waits all run to their bounded timeouts.

So the ADC check is sensitive to the model: a wrong ADC layout fails it.
