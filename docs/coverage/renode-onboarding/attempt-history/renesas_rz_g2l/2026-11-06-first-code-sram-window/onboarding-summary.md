# Initial CPU-only profile run (superseded)

The initial narrowed profile mapped only the M33 non-secure code SRAM at
`0x00010000`. The direct-ELF loop reached 100,000 instructions, but its
initial MSP was outside that SRAM window and the firmware did not use the
stack. This result is retained for traceability; the current profile also maps
M33 data SRAM and uses that stack before the final smoke run.
