# Control: descriptor without the SPI0 model

Same firmware and smoke assertions as `max31855-smoke.yaml`. The chip
descriptor is `configs/chips/atmega4809.yaml` as it was at commit 2022feaa4
(`git show 2022feaa4:configs/chips/atmega4809.yaml`), which has no window at
0x08C0, and no device is attached.

Result: FAIL, 0/3 checks. The run stops with `Memory access violation at
0x8c1` (`run.log`) before any serial output. 0x08C1 is SPI0.CTRLB, the first
register `SPI.begin()` touches. The passing run therefore depends on the
`megaavr_spi` model.
