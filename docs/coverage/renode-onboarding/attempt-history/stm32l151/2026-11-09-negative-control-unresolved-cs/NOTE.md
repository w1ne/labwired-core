Negative control (2026-11-09, round 399): the second SPI1 MAX31855 declared
with `cs_pin: "PB99"`, a label that resolves to no GPIO, so the engine keeps
that device on the pre-CS-routing pass-through (every frame, no edges). Same
CLI build as the passing run. Device B (50 °C, 0x03201600) answers over the
PA4 device on every SPI1 read ("last device wins"), and the frame read after a
two-byte CS-cut comes back rotated (`after_cut=0x16000320`): the MAX31855
re-framed every four bytes instead of restarting on CS↓.
`spi1 shared a=0x03201600 b=0x03201600 after_cut=0x16000320 a=0 b=1 restart=0 FAIL`,
summary 0x15116FFF, smoke FAIL.
