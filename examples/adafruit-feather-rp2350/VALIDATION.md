# Feather RP2350 validation

The onboarding smoke builds the core CLI and `thumbv8m.main-none-eabi`
firmware, then runs the firmware against the narrow Feather system profile.
It requires at least eight output transitions on `sio:7` and stops at the
configured 200,000-step limit.

The profile includes the RP2350 memory geometry, IO_BANK0 pad selection, SIO,
and the active-high red indicator LED on GPIO7. It omits the GPIO21 NeoPixel
and PIO signaling, STEMMA QT I2C, USB, HSTX, optional PSRAM, and all other
header functions. No physical Feather board was used for this smoke.

See the committed run receipts in [`evidence/`](evidence/) and live board
availability in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Hosted RP2350 cross-check

A separate hosted `labwired_verify` attempt on its catalog `rp2350-zero` target
returned `status: unsupported`: the hosted twin stopped on an undecoded Thumb-32
instruction at PC `0x1008`, before firmware reached GPIO7, and captured zero
edges. That target is only a chip-level cross-check, not this Feather profile;
there is no hosted `model_verified` claim for this board. The local repository
smoke above is the board-profile evidence. The narrow follow-up for the hosted
path is to decode the reported Thumb-32 instruction and rerun the GPIO oracle.
