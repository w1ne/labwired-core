# renesas-r7fa8m1a negative control: SCI0 removed (2026-11-08)

Same firmware and smoke script, with `sci0` removed from the chip (the
`renesas-r7fa8m1a.yaml` copy here). FAIL 4/8: no UART bytes, no P600 edges,
and the CCR3/PODR readback words are never written. The MUL result, the
SCICKCR readback (SYSTEM handshake), the MSTPCRB clear and the max_steps
stop reason still hold. The first SCI0 access (the CCR0 write at 0x40358008,
PC 0x020008FC) is unmapped and escalates to a HardFault (precise BusFault,
BFAR 0x40358008), so the UART/GPIO checks depend on the modelled SCI_B.

Earlier the same day the first `--only` run failed 7/9 because the firmware
polled CSR bit 21 for TEND after the banner. R7FA8M1AH.h has
`R_SCI_B0_CSR_TEND_Pos = 30`, which the `sci_b` model already reports; the
firmware constant was wrong, not the model. Fixed before the passing run.
