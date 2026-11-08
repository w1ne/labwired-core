# L8 CAN loopback on the Arduino UNO R4 WiFi

The matrix L8 sketch, RA4M1 branch, on CAN0 (`ra_can`). Like every other
board's L8 cell this is register-level: the Arduino `Arduino_CAN` library
is not used. The branch follows the RA4M1 hardware manual's start-up order:

1. Clear MSTPCRB.MSTPB2. MSTP is a declared stub, so this has no effect in
   the model.
2. Write CTLR = 0x0100 (reset mode, leave sleep) and wait for STR.RSTST = 1
   and SLPST = 0.
3. Write BCR = 0xA0020300.
4. Write CTLR = 0x0200 (halt mode) and wait for HLTST.
5. Write TCR = 0x07 (TSTE, TSTM = 11: internal loopback), MKIVLR =
   0xFFFFFFFF (exact match) and MIER = 0.
6. Load MB0 with standard ID 0x123, DLC 1, data 0xA5, and MB1 with ID 0x123.
7. Write CTLR = 0 (operation mode) and wait for STR mode bits = 0.
8. Write MCTL1 = RECREQ, then MCTL0 = TRMREQ.

It prints `LW_L8_OK` only if MCTL1.NEWDATA sets, MCTL0.SENTDATA is set, and
MB1 holds SID 0x123, DLC 1 and D[0] = 0xA5.

Files:
* `test.yaml` / `result.json` / `uart.log` / `junit.xml`: the matrix cell
  as `run_matrix.py` ran it. It stops once `LW_L8_OK` is seen (156,902
  steps).
* `full.yaml` / `full-result.json` / `full-uart.log`: the same firmware
  run for the full 5,000,000 steps. It asserts that the whole transcript
  is exactly `LW_L8_BOOT` / `LW_L8_OK`.
* `controls/`: the same firmware with CAN0 declared as a zero stub. The
  sketch prints `LW_L8_FAIL`.

What this does not show:
* No bus is attached. A frame completes only in loopback; outside
  loopback a transmission gets no ACK and stays TRMACTIVE.
* The CTX0/CRX0 pads (D10/D13) are not bound.
* Timing is not modelled. BCR is stored only, and a loopback frame
  completes in the MCTL store.
* `Arduino_CAN` (FSP `r_can`) needs mailbox interrupts. `ra_can` refuses
  MIER/EIER, so a sketch using that library fails loudly rather than
  hanging.
* FIFO mailbox mode is refused.
* Masks, mixed/extended ID formats, overwrite/overrun, ID-priority ordering
  and the mailbox search are covered by the `ra_can` unit tests, not by
  firmware.

Paths in the yaml files are repo-relative records. The CLI resolves them
relative to the script, so run them from a copy with absolute paths.
