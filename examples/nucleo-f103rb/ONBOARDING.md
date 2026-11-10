# NUCLEO-F103RB onboarding

This profile targets ST's NUCLEO-F103RB board with its STM32F103RBT6. ST's
live product pages report the board and MCU as **Active**; its online store
showed the board in stock on 2026-11-10. The latest availability receipt is in
[`../nucleo-f103rb-bme280/evidence/vendor-status.md`](../nucleo-f103rb-bme280/evidence/vendor-status.md).

## Modeled board paths

| Board signal | MCU pin / peripheral | Modeled behavior | Smoke |
|---|---|---|---|
| LD2 user LED | PA5 / GPIOA | Active-high output | Toggled; GPIO edge assertion |
| B1 user button | PC13 / GPIOC | Active-low input | Mapped, not read |
| ST-LINK virtual COM port | PA2 TX, PA3 RX / USART2 | USART2 data-register output captured by the simulator | Sends `OK` |
| BME280 STEMMA QT breakout | PB6/SCL, PB7/SDA / I2C1 | Reads BME280 chip-ID register (`0xD0`) at `0x77` | Separate sensor profile; ID `0x60` |

The base system manifest points to `stm32f103rb-nucleo-smoke` and uses RCC,
GPIOA, GPIOC, and USART2. The companion BME280 profile uses GPIOB and I2C1
from the same narrow chip descriptor. The wider `stm32f103rb.yaml` profile
contains zero-answering USB and backup-register windows; both NUCLEO-F103RB
systems omit them and do not claim the rest of the chip's peripheral surface.

The board mapping follows ST [UM1724](https://www.st.com/resource/en/user_manual/um1724-stm32-nucleo64-boards-mb1136-stmicroelectronics.pdf):
LD2 is on PA5, B1 is on PC13, and the default SB13/SB14 configuration routes
USART2 PA2/PA3 to the ST-LINK VCP. I2C1 defaults to PB6/SCL and PB7/SDA. The
simulator captures USART2 output at the MCU; it does not model ST-LINK
firmware, the USB cable, or physical UART signaling.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f103rb \
  --crate firmware-nucleo-f103rb-smoke \
  --target thumbv7m-none-eabi \
  --script examples/nucleo-f103rb/io-smoke.yaml \
  --system configs/systems/nucleo-f103rb.yaml \
  --out-dir examples/nucleo-f103rb/evidence
```

The base smoke's committed evidence contains build logs, the smoke result, UART
transcript, snapshot, metrics, and scoreboard. The run is deterministic
simulator evidence, not a physical-board capture. The button is mapped but not
exercised; see the companion BME280 example for the separate I2C1 smoke. USB,
backup registers, and the other F103 peripherals remain outside these profiles.
