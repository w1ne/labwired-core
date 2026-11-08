# core-v-mcu before the PULP_STDOUT smoke

CPU/memory-only measurement (RV32IMC MUL word at 0x1C000000, no peripherals
mapped) preserved before the chip gained Renode's PULP_STDOUT window at
0x1A10FF80 and the smoke started printing through it.
