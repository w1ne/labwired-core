# Control: descriptor without the TWI0 model

Same firmware and smoke assertions as `bme280-smoke.yaml`. The chip descriptor
is `configs/chips/atmega4809.yaml` from the parent commit (`git show
HEAD~1:configs/chips/atmega4809.yaml`), which has no window at 0x08A0, and no
device is attached.

Result: FAIL, 0/6 checks. The run stops with `Memory access violation at
0x8a3` (`run.log`) before any serial output. 0x08A3 is TWI0.MCTRLA, the first register `Wire.begin()` writes.
The passing run therefore depends on the `megaavr_twi` model.
