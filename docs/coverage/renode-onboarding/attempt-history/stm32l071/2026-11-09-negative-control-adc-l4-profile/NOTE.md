# Negative control: the ADC on the L4 layout

Same firmware, system and scenario as the passing run (summary
0x714FFFFF), with one line of the chip profile changed: the ADC declared
`profile: stm32l4` instead of `profile: stm32l0`. That is the profile the
`configs/chips/stm32l073*` board chip uses today.

Result: FAIL 4/6 checks. The firmware never prints its `adc` line or its
summary (0x20001000 stays at the 0xA5A5A5A5 fill): the L4 model latches
`CR.ADCAL` when there is no kernel clock, so the RM0377 calibration
loop never ends. The L4 block has no CHSELR either, so even past that
point the sequence would not convert.

So the check is sensitive to the model: a wrong ADC layout fails it.
