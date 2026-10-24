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
| `env-spi16-nss.yaml` | F401 SPI1 master, 16-bit frames, NSS driven by the SPI (`SSOE=1`, PA4 AF5) | F401 SPI1 slave, hardware NSS (`SSM=0`, PA4 AF5) | `nss` (pull-up), `sck`, `miso`, `mosi` |
| `env-spi-l4-nss.yaml` | STM32L476 SPI1 master, NSS driven by the SPI (`SSOE=1`, PA4 AF5; `l4-spi-nss-master-nssp.elf`: `NSSP` pulse mode); the tests swap in the STM32G474 (`system-g474.yaml`) | L476 (or G474) SPI1 slave, hardware NSS (`SSM=0`, PA4 AF5) | `nss` (pull-up), `sck`, `miso`, `mosi` |
| `env-i2c-legacy.yaml` | STM32F103 legacy I2C1 master (PB6/PB7) | F401 legacy I2C1 slave at 0x42 | `scl`, `sda` (pull-up) |
| `env-spi-rp2040.yaml` | RP2040 SPI0 master (GP2-GP5, hardware CSn) | G071 SPI1 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-rp2040-slave.yaml` | G071 SPI1 master, mode 3 | RP2040 SPI0 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-i2c-rp2040.yaml` | RP2040 I2C0 controller (GP4 SDA, GP5 SCL) | G071 I2C1 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-i2c-rp2040-target.yaml` | G071 I2C1 controller | RP2040 I2C0 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-spi-nrf52.yaml` | nRF52840 SPIM0 master (P0.03 SCK, P0.04 MOSI, P0.05 MISO, CS on GPIO P0.02), EasyDMA, END interrupt | G071 SPI1 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-nrf52-spim3.yaml` | nRF52840 SPIM3 master with its hardware CSN (P0.02, `CSNPOL`, `IFTIMING.CSNDUR` = 64) and D/CX (P0.06, `DCXCNT` = 1); `nrf-spim3-master-csnpol.elf`: active-high CSN | G071 SPI1 slave, hardware NSS; PA8 reads D/CX | `cs`, `dcx` (pull-up), `sck`, `miso`, `mosi` |
| `env-spi-nrf52-slave.yaml` | G071 SPI1 master, mode 0 | nRF52840 SPIS0 slave (CSN P0.02), semaphore + EasyDMA | `cs`, `sck`, `miso`, `mosi` |
| `env-i2c-nrf52.yaml` | nRF52840 TWIM0 controller (P0.27 SCL, P0.26 SDA), K250, shortcuts | G071 I2C1 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-i2c-nrf52-target.yaml` | G071 I2C1 controller | nRF52840 TWIS0 target at 0x42 (polled `WRITE`/`READ`, `PREPARERX`/`TX`) | `scl`, `sda` (pull-up) |
| `env-spi-esp32c3.yaml` | ESP32-C3 GP-SPI2 master (GPIO4 SCK, GPIO5 MOSI, GPIO3 MISO, GPIO1 CS0 through the GPIO matrix), mode 0, 100 kHz | G071 SPI1 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-esp32c3-slave.yaml` | ESP32-C3 GP-SPI2 slave (W buffer) | G071 SPI1 master | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-dma.yaml` | G071 SPI1 master, both directions by DMA (DMAMUX → DMA1) | G071 SPI1 slave, both directions by DMA | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-esp32c3-dma.yaml` | ESP32-C3 GP-SPI2 master, full duplex by GDMA | G071 SPI1 slave by DMA | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-esp32c3-dma-slave.yaml` | ESP32-C3 GP-SPI2 slave by GDMA | G071 SPI1 master by DMA | `cs`, `sck`, `miso`, `mosi` |
| `env-i2c-esp32c3.yaml` | ESP32-C3 I2C0 controller (GPIO8 SDA, GPIO9 SCL), 100 kHz | G071 I2C1 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-i2c-esp32c3-target.yaml` | ESP32-C3 I2C0 target at 0x42, stretching SCL until its firmware answers | G071 I2C1 controller | `scl`, `sda` (pull-up) |
| `env-spi-esp32s3.yaml` | ESP32-S3 GP-SPI2 master (GPIO12 SCK, GPIO11 MOSI, GPIO13 MISO, GPIO10 CS0 through the GPIO matrix), mode 0, 100 kHz | G071 SPI1 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-esp32s3-slave.yaml` | ESP32-S3 GP-SPI2 slave (W buffer) | G071 SPI1 master | `cs`, `sck`, `miso`, `mosi` |
| `env-i2c-esp32s3.yaml` | ESP32-S3 I2C0 controller (GPIO8 SDA, GPIO9 SCL), 100 kHz | G071 I2C1 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-i2c-esp32s3-target.yaml` | ESP32-S3 I2C0 target at 0x42, stretching SCL until its firmware answers | G071 I2C1 controller | `scl`, `sda` (pull-up) |
| `env-spi-esp32.yaml` | classic ESP32 VSPI master (GPIO18 SCK, GPIO23 MOSI, GPIO19 MISO, GPIO5 CS0 through the GPIO matrix), mode 0, 100 kHz | G071 SPI1 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-esp32-slave.yaml` | classic ESP32 HSPI slave (W buffer), same pads | G071 SPI1 master | `cs`, `sck`, `miso`, `mosi` |
| `env-i2c-esp32.yaml` | classic ESP32 I2C0 controller (GPIO21 SDA, GPIO22 SCL), 100 kHz | G071 I2C1 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-i2c-esp32-target.yaml` | classic ESP32 I2C0 target at 0x42, TX FIFO kept loaded ahead of reads | G071 I2C1 controller | `scl`, `sda` (pull-up) |
| `env-spi-esp32s3-spi3.yaml` | ESP32-S3 GP-SPI3 master on bank-1 pads (GPIO36 SCK, GPIO35 MOSI, GPIO37 MISO, GPIO34 CS0) | G071 SPI1 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-esp32s3-spi3-slave.yaml` | ESP32-S3 GP-SPI3 slave, same pads | G071 SPI1 master | `cs`, `sck`, `miso`, `mosi` |
| `env-i2c-esp32s3-i2c1.yaml` | ESP32-S3 I2C1 controller on bank-1 pads (GPIO41 SDA, GPIO42 SCL) | G071 I2C1 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-i2c-esp32s3-i2c1-target.yaml` | ESP32-S3 I2C1 target at 0x42, same pads | G071 I2C1 controller | `scl`, `sda` (pull-up) |
| `env-i2c-esp32-i2c1.yaml` | classic ESP32 I2C1 controller (0x3FF6_7000) on GPIO32 SDA, GPIO33 SCL | G071 I2C1 target at 0x42 | `scl`, `sda` (pull-up) |
| `env-i2c-esp32-i2c1-target.yaml` | classic ESP32 I2C1 target at 0x42, same pads | G071 I2C1 controller | `scl`, `sda` (pull-up) |
| `env-spi-esp32-hi.yaml` | classic ESP32 VSPI master, GPIO33 SCK, GPIO32 MOSI, GPIO34 MISO (input-only pad), GPIO5 CS0 | G071 SPI1 slave | `cs`, `sck`, `miso`, `mosi` |
| `env-spi-esp32-hi-slave.yaml` | classic ESP32 HSPI slave, SCK / MOSI / CS0 in on the input-only GPIO34 / GPIO35 / GPIO36, MISO out on GPIO33 | G071 SPI1 master | `cs`, `sck`, `miso`, `mosi` |
| `env-gpio-esp32-esp32s3.yaml` | classic ESP32, plain GPIO: out GPIO32, in GPIO39 (input-only) | ESP32-S3, plain GPIO: in GPIO40, out GPIO41 | `a2b`, `b2a` (pull-down) |
| `env-rtc-gpio-esp32-esp32s3.yaml` | classic ESP32, RTC GPIO (RTC_IO `MUX_SEL` set): open-drain out GPIO25 (RTC_GPIO6), in GPIO26 (RTC_GPIO7); GPIO27 handed from the RTC domain to a digital output | ESP32-S3, RTC GPIO: in GPIO4 with `RUE`, out GPIO5, in GPIO6 with `RDE` | `a2b`, `b2a`, `handoff` (no board pull) |

SPI: the master sends `A5 3C 5A C3` and receives `81 82 83 84`, which the
slave queues from its SPI interrupt (or preloads in its FIFO, on the RP2040).
The tests also swap in the frame-shape variants (`spi-*-mode1/2/3`,
`spi-*-lsb`), the G071's other `CR2.DS` frame sizes (`spi-*-ds16/12/4`, each
chip as master in turn), a slave that feeds every answer from its TXE
interrupt (`spi-slave-txe`), one that underruns (`spi-slave-underrun`) and a
master that leaves `CR2.FRXTH` at reset (`spi-master-nofrxth`), which the
G0's FIFO SPI stalls after one 8-bit frame as silicon does.

SPI by DMA: the DMA pairs exchange eight bytes, `A5 3C 5A C3 0F F0 81 7E`
one way and `81 82 .. 88` the other, with no CPU access to `DR` or the W
buffer (`src/spi_dma_*.c` on the G071, `src/esp32c_spi_dma_*.c` on the C3
and the C6, the same sources built with `-DCHIP_C6`).

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
the same register-file exchange on its I2C0, as controller and as target. The ESP32-C3
runs both exchanges as well (`src/esp32c_*.c`, register level), and so does
the ESP32-C6 from the same sources (`-DCHIP_C6`, `firmware/esp32c6-*.elf`;
the tests swap the system and firmware into the `esp32c3` environments).
`esp32c3-esp-hal/` holds esp-hal 1.1 SPI and I2C master applications that run
the same exchanges through esp-hal's own drivers. The ESP32-S3 and the classic
ESP32 run them from hand-written Xtensa assembly (`src/esp32x_*.S`,
`src/esp32x.inc`; `--defsym CHIP_ESP32=1` for the classic part,
`--defsym INST1=1` for the S3's GP-SPI3 / I2C1 and the classic I2C1,
`--defsym PADS_HI=1` for bank-1 pads): LLVM 18's
Xtensa backend assembles but does not compile C or link, so `build.sh` runs
`llvm-mc` and `src/xtensa_elf.py` wraps the relocation-free object into an
ELF loaded at IRAM.

Results are left in SRAM at 0x20000100 (see the comment at the top of each
source). The tests are
`cargo test -p labwired-core --test world_gpio_net_buses`.

## Rebuild the firmware

```bash
examples/gpio-net-buses/build.sh   # clang + lld, no vendor SDK
```

The ELFs are committed so the tests run without a toolchain.
