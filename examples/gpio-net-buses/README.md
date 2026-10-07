# SPI and I²C between two chips, over gpio nets

Two STM32G071s in one world whose hardware SPI1 (and, in the second
environment, I2C1) are wired pin to pin with `gpio_net` interconnects (see
[GPIO nets](../../docs/howto/gpio-nets.md#buses-over-nets)). Nothing is
exchanged byte by byte behind the scenes: every bit crosses the nets at the
pads, so the net reports and a logic analyzer on either board see the real
waveform, and the bus behaves like wires: ACKs are SDA held low by the other
chip, a target that is slow to answer holds SCL low, a missing target is the
pull-up reading 1.

| Environment | Node `a` | Node `b` | Nets |
|-------------|----------|----------|------|
| `env-spi.yaml` | SPI1 master, mode 0, SCK = f/128, CS on a GPIO (PA4) | SPI1 slave, hardware NSS (PA4), RXNE interrupt | `cs` (pull-up), `sck`, `miso`, `mosi` (pull-down) |
| `env-i2c.yaml` | I2C1 controller (PB6/PB7, open drain) | I2C1 target at 0x42, interrupt driven | `scl`, `sda` (pull-up) |

SPI: the master sends `A5 3C 5A C3` and receives `81 82 83 84`, which the
slave queues from its SPI interrupt.

I²C: the controller writes `10 DE AD` to 0x42 (register pointer 0x10, then two
bytes), writes the pointer again and reads two bytes back through a repeated
START (`DE AD`), then writes to 0x50, where nobody answers: NACKF and STOP.

Results are left in SRAM at 0x20000100 (see the comment at the top of each
source). The tests are
`cargo test -p labwired-core --test world_gpio_net_buses`.

## Rebuild the firmware

```bash
examples/gpio-net-buses/build.sh   # clang + lld, no vendor SDK
```

The ELFs are committed so the tests run without a toolchain.
