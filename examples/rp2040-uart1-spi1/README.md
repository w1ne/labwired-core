# RP2040 UART1 and SPI1

Register-level smoke for the second UART and the second SPI on the RP2040
(Raspberry Pi Pico silicon), plus the UART interrupt lines.

| Step | Peripheral | Check |
|------|------------|-------|
| 1 | RESETS | Release UART0, UART1 and SPI1; wait for `RESET_DONE`. |
| 2 | UART1 (0x40038000) | 115200 8N1; prints `UART1 OK`. |
| 3 | SPI1 (0x40040000) | PL022 loopback (`SSPCR1.LBM`): 0xA5 and 0x3C come back. |
| 4 | UART0_IRQ (NVIC 20) | `UARTIMSC.TXIM` with an empty TX FIFO; the handler runs once. |
| 5 | UART1_IRQ (NVIC 21) | `UARTIMSC.RXIM`; an injected `R` on UART1 RX reaches the handler, which echoes it. |
| 6 | UART0 | Prints `RP2040 UART1 SPI1 PASS`. |

SRAM 0x20000100 holds the results: `[0]` and `[1]` the SPI1 bytes, `[2]` the
UART0 interrupt count, `[3]` the UART1 byte, `[4]` 0xA55A when done, `[5]` 1
on pass.

Arduino-Pico `Serial2` / `SPI1`, MicroPython `UART(1)` / `SPI(1)` and
pico-sdk `uart1` / `spi1` use these blocks. Before this example the chip
descriptor mapped neither block and wired no UART interrupt line.

## Run it

```bash
labwired test --script examples/rp2040-uart1-spi1/smoke.yaml --output-dir out
```

`./build.sh` rebuilds `firmware/uart1-spi1.elf` (arm-none-eabi-gcc).

Model limits: the SPI is the behavioural PL022 model (no SSPIMSC interrupt
line); the PL011 model has no FIFO-level thresholds or RX timeout interrupt.
