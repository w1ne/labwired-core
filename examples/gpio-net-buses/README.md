# SPI and I²C between two chips, over gpio nets

Pairs of chips in one world whose hardware SPI or I²C are wired pin to pin
with `gpio_net` interconnects (see
[GPIO nets](../../docs/howto/gpio-nets.md#buses-over-nets)). Nothing is
exchanged byte by byte behind the scenes: every bit crosses the nets at the
pads, so the net reports and a logic analyzer on either board see the real
waveform, and the bus behaves like wires: ACKs are SDA held low by the other
chip, a target that is slow to answer holds SCL low, a missing target is the
pull-up reading 1.

| Environment | Node `a` (or `rp`) | Node `b` (or `rp`) | Nets |
|-------------|----------|----------|------|
| `env-spi.yaml` | STM32G071 SPI1 master, mode 0, SCK = f/128, CS on a GPIO (PA4) | G071 SPI1 slave, hardware NSS (PA4), RXNE interrupt | `cs` (pull-up), `sck`, `miso`, `mosi` (pull-down) |
| `env-i2c.yaml` | G071 I2C1 controller (PB6/PB7, open drain) | G071 I2C1 target at 0x42, interrupt driven | `scl`, `sda` (pull-up) |
| `env-spi16.yaml` | STM32F401 SPI1 master, 16-bit frames | F401 SPI1 slave, software select | `sck`, `miso`, `mosi` |
| `env-i2c-legacy.yaml` | STM32F103 legacy I2C1 master (PB6/PB7) | F401 legacy I2C1 slave at 0x42 | `scl`, `sda` (pull-up) |
| `env-spi-rp2040.yaml` | RP2040 SPI0 master (GP2-GP5, hardware CSn) | G071 SPI1 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-rp2040-slave.yaml` | G071 SPI1 master, mode 3 | RP2040 SPI0 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-i2c-rp2040.yaml` | RP2040 I2C0 controller (GP4 SDA, GP5 SCL) | G071 I2C1 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-i2c-rp2040-target.yaml` | G071 I2C1 controller | RP2040 I2C0 target at 0x42 | `scl`, `sda` (pull-up) |

SPI: the master sends `A5 3C 5A C3` and receives `81 82 83 84`, which the
slave queues from its SPI interrupt (or preloads in its FIFO, on the RP2040).
The tests also swap in the frame-shape variants (`spi-*-mode1/2/3`,
`spi-*-lsb`), a slave that feeds every answer from its TXE interrupt
(`spi-slave-txe`) and one that underruns (`spi-slave-underrun`).

I²C: the controller writes `10 DE AD` to 0x42 (register pointer 0x10, then two
bytes), writes the pointer again and reads two bytes back through a repeated
START (`DE AD`), then writes to 0x50, where nobody answers: NACKF and STOP.
The legacy pair adds a single-byte read. `i2c-bulk-*` (on the `env-i2c.yaml`
wiring) moves 300 bytes each way over the 10-bit address 0x2A5 with
`RELOAD`, then a general call and a write to a masked `OAR2` address. The
tests also run the legacy pair the other way round (F401 master, F103 slave)
and a G071 controller against the F103 slave, and swap in targets built with
`-DSLOW=1` (`*-target-slow.elf`), whose firmware answers several SCL low
periods late, so the controller is held by clock stretching. The RP2040 runs
the same register-file exchange on its I2C0, as controller and as target.

Results are left in SRAM at 0x20000100 (see the comment at the top of each
source). The tests are
`cargo test -p labwired-core --test world_gpio_net_buses`.

## Rebuild the firmware

```bash
examples/gpio-net-buses/build.sh   # clang + lld, no vendor SDK
```

The ELFs are committed so the tests run without a toolchain.
