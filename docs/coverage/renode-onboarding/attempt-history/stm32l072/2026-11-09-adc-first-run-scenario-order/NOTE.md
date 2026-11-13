# First ADC run: a scenario error, not a model error

The first L072 run with the ADC check FAILED 5/6: every check printed `ok`
and the summary was the full 0x727FFFFF, but the scenario listed the `adc`
line after `rng`. The firmware runs the ADC check right after `crc`
(summary bit 22), so the `uart_ordered` assertion failed. The scenario
order was fixed to match the firmware; the model and firmware did not
change between this run and the passing one.
