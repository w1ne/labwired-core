# nRF54L15 smart-ring — bare-metal C I²C sensor probe

Reference firmware for the [smart-ring reference system](../../configs/systems/smart-ring.yaml).
Boots the [nRF54L15 board profile](../../docs/boards/nrf54l15.md), brings up
TWIM21 (I²C master, EasyDMA) on the ring's sensor bus, and does a real
register read of each of the four sensors' identity register — proving the
firmware genuinely drives the sensor models over I²C, not just that it boots.

Extends [`../nrf54l15-dk`](../nrf54l15-dk) (boot + UART banner) with an I²C
bring-up. **No Zephyr, no nRF Connect SDK, no CMSIS** — the only requirement is
`arm-none-eabi-gcc`.

## The bus (matches `smart-ring.yaml`)

TWIM21 @ `0x500C7000`, SCL = P1.02, SDA = P1.03.

| id     | part      | addr | id register        | expected      |
|--------|-----------|------|--------------------|---------------|
| imu    | BMI270    | 0x68 | 0x00 `CHIP_ID`     | `0x24`        |
| ppg    | MAX30102  | 0x57 | 0xFF `PART_ID`     | `0x15`        |
| temp   | TMP117    | 0x48 | 0x0F `DEVICE_ID`   | `0x0117` (16-bit BE) |
| haptic | DRV2605   | 0x5A | 0x00 `STATUS`      | `0xE0` (DEVICE_ID = 7 in bits[7:5]) |

## Build

```sh
make            # build build/nrf54l15-smart-ring.elf
make publish    # build and copy to ../../tests/fixtures/ (the committed fixture)
make clean
```

Toolchain on macOS: `brew install arm-none-eabi-gcc`.

## Run

```sh
cargo build -p labwired-cli --release
./target/release/labwired \
    --firmware examples/nrf54l15-smart-ring/build/nrf54l15-smart-ring.elf \
    --system configs/systems/smart-ring.yaml \
    --max-steps 500000
```

Expected output:

```
smart-ring nRF54L15 I2C sensor probe
TWIM21@0x500C7000 SCL=P1.02 SDA=P1.03
imu    BMI270    addr=0x68 reg=0x00 -> id=0x24 ack=Y [OK]
ppg    MAX30102  addr=0x57 reg=0xff -> id=0x15 ack=Y [OK]
temp   TMP117   addr=0x48 reg=0x0f -> id=0x0117 ack=Y [OK]
haptic DRV2605   addr=0x5a reg=0x00 -> id=0xe0 ack=Y [OK]
probe done
```

The `id=` field is the byte(s) the firmware actually read back over TWIM. The RX
buffer is seeded with `0xEE` before each transaction, so a value matching the
datasheet ID is proof the transaction reached the modelled slave rather than a
stub. `ack=Y` is derived from `ERRORSRC.ANACK` / `EVENTS_ERROR`: an unpopulated
address NACKs, so `ack=Y` means the model acknowledged its address on the bus.

## The TWIM register-read sequence

Each read is the canonical write-pointer / repeated-START / read / STOP,
driven entirely by shorts so the CPU never intervenes between the two legs:

```
ADDRESS      = <7-bit addr>
SHORTS       = LASTTX_DMA_RX_START (1<<7) | LASTRX_STOP (1<<12)
DMA.TX.PTR   = &reg_ptr ; DMA.TX.MAXCNT = 1
DMA.RX.PTR   = &rx_buf  ; DMA.RX.MAXCNT = n
TASKS_DMA.TX.START = 1        -> TX (reg ptr) --short--> RX (id) --short--> STOP
poll EVENTS_STOPPED
```

This is the nRF54L-generation TWIM layout (EasyDMA in a `DMA.{RX,TX}` cluster),
NOT the nRF52 TWIM at a new base — see `src/nrf54l15.h` and
`crates/core/src/peripherals/nrf54l/twim.rs`.

## Validation

- `crates/core/tests/nrf54l15_smart_ring_probe.rs` — boots this ELF against
  `smart-ring.yaml` and asserts each sensor's real WHO_AM_I byte(s) come back
  over I²C, with no NACK, no mismatch, and no surviving `0xEE` sentinel.

## Files

```
Makefile          arm-none-eabi build (with -MMD -MP header deps)
nrf54l15.ld       RRAM 1524K @ 0x0, RAM 256K @ 0x20000000
src/startup.c     ARMv8-M vector table, .data/.bss init, Reset_Handler
src/nrf54l15.h    hand-checked UARTE20 + TWIM21 register subset (not CMSIS)
src/main.c        UARTE banner + TWIM21 four-sensor WHO_AM_I probe
```
