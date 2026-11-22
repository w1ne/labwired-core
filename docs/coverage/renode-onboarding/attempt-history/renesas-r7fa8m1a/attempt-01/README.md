# R7FA8M1A attempt 01

The first exact-import simulation reached the CLI but did not execute firmware:
`result.json` reports `config_error`, 0 instructions, and the chip parser reports
that the descriptor omitted the required `peripherals` field. This is a
configuration failure, not boot evidence. The active descriptor now supplies an
empty peripheral list because this smoke claims CPU and memory only.
