# STM32F103 CAN dashboard (three Blue Pills on one bus)

"One wrong number takes down a car network." Three STM32F103 nodes on a
500 kbit/s CAN bus, in plain C at register level (no HAL, no libc):

| node | firmware | does |
|---|---|---|
| engine | `firmware/engine.c` | throttle potentiometer on PA0 (ADC1 ch0) -> CAN `0x100`, 2 bytes RPM big-endian, every 10 ms. `rpm = 800 + adc * 6200 / 4095`. TIM2 interrupt, main loop `WFI`. |
| dashboard | `firmware/dashboard.c` | filters `0x100` and `0x200` into FIFO0, RX0 interrupt. SSD1306 128x64 on I2C1 (PB6/PB7, `0x3C`): big `RPM 3200`, a bar, `TEMP 21.5 C` (`TEMP --` until received), `CAN OK` or its own ESR state. 10 Hz refresh, only changed pages are sent. USART1 status line once a second. |
| sensor | `firmware/sensor.c` | CAN `0x200` every 100 ms, temperature x10 (a synthetic 21.0-23.0 degC triangle, not a measurement). One constant, `CAN_BITRATE`, sets BRP. ABOM off: bus-off is permanent. PC13 LED blinks at 1 Hz, 5 Hz when `ESR.BOFF`. USART1 reports state/TEC/REC. |

Every node: CAN1 on PA11/PA12, 72 MHz from 8 MHz HSE (PLL x9), APB1 36 MHz,
BTR = BRP 3, TS1 14 tq, TS2 3 tq (36e6 / 4 / 18 = 500 kbit/s), automatic
retransmission on.

The sensor is built twice: `sensor_500k.elf` (fixed) and `sensor_250k.elf`
(the field bug: BRP 7, 250 kbit/s on a 500 kbit/s bus).

## Build

```bash
make -C examples/f103-can-dashboard/firmware install   # arm-none-eabi-gcc; copies to prebuilt/
```

`prebuilt/` holds the ELFs the tests load.

## Run

`world-fixed.yaml` and `world-bug.yaml` are the two worlds (run from the repo
root). The proof is `crates/core/tests/world_can_dashboard_demo.rs`:

```bash
cargo test --release -p labwired-core --features event-scheduler \
  --test world_can_dashboard_demo -- --include-ignored --nocapture
```

What happens with the 250 kbit/s sensor: at 10 ms it reads the engine's first
frame at the wrong rate and, error-active, destroys it with error flags (the
engine retransmits, ~15 attempts); at ~11 ms it is error-passive and stops
disturbing the bus; at 100 ms it transmits its own first frame, every attempt
fails (TEC +8), and at ~102.6 ms it is bus-off (TEC 248, `ESR.BOFF`). From
then on the engine's traffic is ok and the dashboard keeps showing RPM; it
never shows a temperature.
