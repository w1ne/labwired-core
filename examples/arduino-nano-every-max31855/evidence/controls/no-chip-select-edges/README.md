# Control: no chip-select edge at the port write

`cargo test -p labwired-core --test avr nano_every_max` with one line of
`crates/core/src/cpu/avr/avrxt.rs` disabled: the `bus.sync_parking_spi_cs()`
call that follows a PORTx.DIR / OUT write. The SPI0 model then sees the
chip-select level only when a byte completes, so the CS high time between
the two 32-bit frames is lost.

Result: 2 of 4 tests FAIL (`cargo-test.log`). The first frame still decodes
(`TC=25.00 C`), and the second prints `CJ=-0.0625 C` in place of
`CJ=22.0000 C`: the converter was not released, so it did not restart its
frame. The passing tests therefore depend on the chip-select edge between
frames.
