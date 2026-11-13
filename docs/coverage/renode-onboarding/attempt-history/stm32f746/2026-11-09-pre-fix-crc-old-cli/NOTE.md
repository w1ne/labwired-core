Pre-fix run (2026-11-09): this round's firmware and descriptor (SYSCFG on
profile stm32f4 and no rcc_ahb1enr_reset, which that binary does not know)
on target/release/labwired built 2026-11-08 15:24, before this round's CRC
fix. crc16=0x0000FF39, poly16=0: a byte store to CRC_DR overwrote DR
instead of feeding 8 bits, so HAL's byte-format CRC was wrong. That binary
also predates later lane commits (RNG draws, round 403; SPI changes), so its
spi/rng lines are not this round's evidence — only the crc line is.
