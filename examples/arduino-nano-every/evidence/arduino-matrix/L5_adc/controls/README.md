# L5 negative control: A0 at a quarter of VDD

Same `firmware.elf` as the passing L5 cell. The only change is the board
(`arduino-nano-every-a0-1250mv.yaml`): A0 (PD3 = AIN3) is held at 1250 mV
instead of 2500 mV.

Result: FAIL. The sketch prints `LW_L5_FAIL` and `LW_L5_VAL 256 256`
(`uart.log`): 1250 mV * 1024 / 5000 mV = 256 against the VDD reference that
`init()` selects. The passing cell's 512 is therefore the converted level on
AIN3, not a fixed mid-scale code.
