# L8 negative control: CAN0 as a zero stub

This uses the same `firmware.elf` as the passing L8 cell. The only change
is the chip profile (`r7fa4m1ab-uno-r4-wifi-can-stub.yaml`): `can0` is
`type: "stub"` instead of `ra_can`. `arduino-uno-r4-wifi-can-stub.yaml` is
the matrix system pointed at that chip.

Result: FAIL. The sketch prints `LW_L8_BOOT` / `LW_L8_FAIL` (`uart.log`)
over the full 5,000,000 steps. The wait for STR.RSTST never ends, because
a stub reads 0. The FAIL line has no `no_can` suffix, which shows the
RA4M1 branch was compiled in. The passing cell's `LW_L8_OK` therefore
depends on the CAN0 model.
