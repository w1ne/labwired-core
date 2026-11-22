First run with GPIO CS routing (2026-11-09, round 399), before the firmware
parked PB0 high ahead of the SPI checks. The shared-CS check passed, but the
earlier SPI1 polled and DMA checks read device B's frame (0x03201600): PB0
was still a reset-state input, which the engine cannot tell from hardware
NSS, so device B stayed on the bus and answered last. Fixed in the firmware
(`cs_output(GPIOB, 0)` before any SPI1 use), the way real firmware parks
every CS high before it touches a shared bus. summary 0x15136FFF, smoke FAIL.
