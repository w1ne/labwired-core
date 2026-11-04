# STM32H573I-DK validation record

The smoke is run with the command in `README.md`. It must boot the bare-metal
Cortex-M33 firmware, enable GPIOI, configure PI9, and record at least 16 output
edges on the active-low LD1 pin before the simulator reaches its step limit.
Committed build and simulation evidence is under `evidence/`.

This is local simulator evidence. It is not a hosted `labwired_verify` oracle
verdict or a hardware confirmation. The profile leaves TrustZone, board power
sequencing, and all external devices outside the tested scope.
