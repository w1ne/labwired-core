# STM32L071 current-base smoke recheck

The original committed L071 smoke and firmware were rerun at the current branch
head and reproduced the SPI1/SPI2 IRQ failure. The committed metric predates
`920338a4c` (`feat(spi): model STM32 TXE and RXNE IRQ levels`): its smoke read
all SPI receive data before sampling the now level-triggered RXNE interrupt.
The UART RX change is absent from this run. The exact current-base failure
artifacts are preserved here; the model's level behavior is exercised by the
updated smoke in the current round.
