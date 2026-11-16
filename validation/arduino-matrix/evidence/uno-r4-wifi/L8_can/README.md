# L8 CAN loopback on the Arduino UNO R4 WiFi

The matrix L8 sketch, RA4M1 branch, on CAN0 (`ra_can`). Like every other
board's L8 cell it is register-level; the `Arduino_CAN` library is not
used. The branch follows the RA4M1 hardware manual's start-up order:

1. Clear MSTPCRB.MSTPB2 (MSTP is a declared stub, so this is a no-op).
2. CTLR = 0x0100 (reset mode, leave sleep); wait STR.RSTST = 1, SLPST = 0.
3. BCR = 0xA0020300.
4. CTLR = 0x0200 (halt mode); wait HLTST.
5. TCR = 0x07 (TSTE, TSTM = 11: internal loopback), MKIVLR = 0xFFFFFFFF
   (exact match), MIER = 0.
6. MB0 = standard ID 0x123, DLC 1, data 0xA5; MB1 = ID 0x123.
7. CTLR = 0 (operation mode); wait for the STR mode bits to clear.
8. MCTL1 = RECREQ, then MCTL0 = TRMREQ.

It prints `LW_L8_OK` only if MCTL1.NEWDATA sets, MCTL0.SENTDATA is set and
MB1 holds SID 0x123, DLC 1 and D[0] = 0xA5.

Files:
* `test.yaml` / `result.json` / `uart.log` / `junit.xml`: the cell as
  `run_matrix.py --boards uno-r4-wifi` ran it (pass, 156,902 steps; stops
  once `LW_L8_OK` is seen). Paths are rewritten repo-relative.
* `controls/`: the same `firmware.elf` with `can0` switched to
  `type: "stub"` in `configs/chips/ra4m1.yaml`. Result: fail after the
  full 5,000,000 steps, UART `LW_L8_BOOT` / `LW_L8_FAIL` (the STR.RSTST
  wait never ends because a stub reads 0). The FAIL line has no `no_can`
  suffix, so the RA4M1 branch was compiled in, and the pass depends on the
  CAN0 model.

What this does not show:
* No bus is attached. A frame completes only in loopback; outside loopback
  a transmission gets no ACK and stays TRMACTIVE (ECSR.AEF set).
* The CTX0/CRX0 pads (D10/D13) are not bound.
* Timing is not modelled. BCR is stored only, and a loopback frame
  completes in the MCTL store.
* `Arduino_CAN` (FSP `r_can`) needs mailbox interrupts. `ra_can` refuses
  MIER/EIER, so a sketch using that library fails loudly instead of hanging.
* FIFO mailbox mode is refused.
* Masks, mixed/extended ID formats, overwrite/overrun, ID-priority ordering
  and the mailbox search are covered by the `ra_can` unit tests, not by
  firmware.
