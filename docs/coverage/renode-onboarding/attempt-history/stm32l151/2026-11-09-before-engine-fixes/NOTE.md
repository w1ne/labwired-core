First run of the extended smoke (2026-11-09), before the two engine fixes,
with an earlier firmware revision (one-byte RXNEIE probe):

- `spiN ... pend=0 FAIL` on all three: the classic STM32 SPI raised RXNEIE
  only for a gpio-net master, never for a slave on `external_devices`.
- `i2c1 ... nack_er=0 FAIL`: AF was set under ITERREN, but in scheduler mode
  the F1 I2C event chain never emitted the ER line (only the legacy walk did).
- `dma1 spi1 frame=0x90160001 ... FAIL`: the one-byte RXNEIE probe left the
  MAX31855 one byte into its frame — the engine does not deliver GPIO CS
  edges to STM32 SPI slaves, so the device re-frames every four bytes. The
  smoke now clocks whole frames only; CS routing is recorded as an open gap.
