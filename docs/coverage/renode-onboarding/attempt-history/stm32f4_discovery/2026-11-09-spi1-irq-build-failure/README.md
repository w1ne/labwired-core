# First SPI1 IRQ fixture attempt

The target-only inventory run stopped in `build_firmware` before firmware execution. The smoke source used `SPI_SR` at two polling sites while the declared status pointer is `SPI1_SR`. The compiler log and failure metrics are retained here. No boot or smoke result was produced by this attempt.
