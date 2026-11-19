# L8 CAN loopback on the NUCLEO-G474RE

The matrix L8 sketch, G474 branch, on FDCAN1 (shared `fdcan` model). Like
every other board's L8 cell it is register-level; the HAL FDCAN driver is
not used. The branch is the H5/G0B1 `enter_loopback` sequence with the G4
addresses from `stm32g474xx.h`:

1. RCC_APB1ENR1 (0x40021058) |= FDCANEN (bit 25).
2. CCCR = INIT|CCE, then + TEST|MON; TEST = LBCK (internal loopback).
3. TX element 0 at SRAMCAN + 0x278 (0x4000A678): std ID 0x123, DLC 1,
   data 0xA5.
4. CCCR = 0xA2 (leave INIT), TXBAR = 1.
5. Spin until RXF0S.F0FL != 0, then read RX FIFO0 element 0 at
   SRAMCAN + 0xB0.

It prints `LW_L8_OK` only if the element holds SID 0x123 and data byte 0xA5.

SRAMCAN on the G474 is at 0x4000A400, 0x4000 above the FDCAN1 registers.
`configs/chips/stm32g474re.yaml` maps it with a same-backing
`memory_regions` mirror onto the model's message RAM (window + 0x800), the
pattern `stm32g0b1re.yaml` already uses.

Files:
* `test.yaml` / `result.json` / `uart.log` / `junit.xml`: the cell as
  `run_matrix.py --boards stm32g474re` ran it with `ststm32@19.7.1` (the
  CI pin). Pass, 176,053 steps; stops once `LW_L8_OK` is seen. Paths are
  rewritten repo-relative. The sibling directories hold the other eight
  cells of the same run (9 pass / 0 skip / 0 fail, see
  `../scoreboard-2026-11-16.md`).
* `controls/`: the same sketch against the `origin/main` chip YAML (no
  `fdcan1`, no `sramcan` mirror). Result: fail after the full 20,000,000
  steps, UART `LW_L8_BOOT` only. The pass depends on the FDCAN1 entry.

What this does not show:
* No bus is attached. A frame completes only in loopback.
* FDCAN2 (0x40006800) reads 0. FDCAN3 (0x40006C00) is not usable: the
  model's message RAM occupies that window.
* FDCAN1_IT1 (IRQ 22) routing through ILS is not modelled.
* Bit timing is not modelled; DBTP/NBTP/CKDIV are storage only.
* No G474 silicon capture backs this block. The register behaviour is the
  H5 NUCLEO-H563ZI capture13 pin; the G4 IP and SRAMCAN layout are the same
  (`stm32g4xx_hal_fdcan.c` SRAMCAN_* constants).

Local toolchain note: with the local `ststm32` 20.0.0, L4 fails on this
row (`SPI.begin()` leaves the SPI instance NULL; PC loops in
`spi_transfer`). That is a core-package defect, the same on `origin/main`
without this change. CI pins 19.7.1; this evidence uses 19.7.1.

Vendor status, read live on 2026-11-16 in a browser tab:
* https://www.st.com/en/microcontrollers-microprocessors/stm32g474re.html — ACTIVE
* https://www.st.com/en/evaluation-tools/nucleo-g474re.html — ACTIVE
