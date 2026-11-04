# EFR32MG1 attempt 01

The first run used the EFR32MG13 fixture, whose linker placed the initial stack
in SRAM at `0x21000000`. Renode's imported EFR32MG1 map places 32 KiB SRAM at
`0x20000000`, so the run faulted on the first instruction with a memory access
violation at `0x21001fe0` and executed zero firmware instructions.

This attempt is retained as a failed run, not boot evidence. The retry uses a
dedicated EFR32MG1 linker map and its exact imported system.
