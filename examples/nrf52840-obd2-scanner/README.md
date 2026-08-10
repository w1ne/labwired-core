# Virtual nRF52840 OBD-II Scanner

This two-node lab runs the real bare-metal nRF52840 scanner ELF against the
real STM32F103 ECU ELF. The scanner talks to an MCP2515 on SPIM2, paints an
SSD1306 on TWIM0, and transmits its state with the nRF52840 RADIO. The MCP2515
and bxCAN1 endpoints share a 500 kbit/s virtual classical-CAN bus.

Build and run from `core/`:

```sh
cargo build -p firmware-nrf52840-obd2-scanner --release --target thumbv7em-none-eabi
make -C examples/nrf52840-obd2-scanner/ecu/firmware
cargo test -p labwired-core --test e2e_nrf52840_obd2_scanner -- --nocapture
cargo run -q -p labwired-cli -- environment-test \
  --env examples/nrf52840-obd2-scanner/env.yaml --max-steps 12000000
```

## Wiring

| Scanner signal | nRF52840 pin/peripheral | Virtual device |
| --- | --- | --- |
| CAN CS | P0.12 / SPIM2 | MCP2515 CS |
| CAN IRQ | P0.11 | MCP2515 active-low IRQ |
| CAN SCK/MOSI/MISO | P0.13/P0.14/P0.15 / SPIM2 | MCP2515 SPI |
| OLED SDA/SCL | P0.26/P0.27 / TWIM0 | SSD1306 at `0x3c` |

The ECU accepts functional requests on `0x7DF` and responds on `0x7E8`.
Mode 01 supplies supported PIDs, 3000 RPM, 88 km/h, and coolant byte 130
(90 °C). Mode 03 initially reports P0133 and U0123. Mode 04 clears them and a
subsequent Mode 03 confirms an empty DTC store. Mode 09 PID 02 returns VIN
`LWOBD2SIM00000001` over ISO-TP.

The nine-byte raw-radio telemetry payload is `version, status, rpm_le,
speed_kph, coolant_celsius, dtc_count, generation_le`. After clearing DTCs its
first seven bytes are `01 01 b8 0b 58 5a 00`.

## Fidelity boundary

Modeled and gated here are real Cortex-M instruction execution, nRF EasyDMA
SPIM/TWIM transactions, MCP2515 standard-ID TX/RX buffers, filters and IRQ,
shared CAN delivery, bxCAN registers, ISO-TP assembly, SSD1306 writes, and raw
nRF RADIO packet transmission. Extended CAN, CAN FD, MCP2515 one-shot mode,
CAN arbitration/error confinement, analog transceiver behavior, wiring faults,
oscillator tolerance, antenna propagation, and standards-compliant BLE GAP
advertising are deferred. Passing this virtual gate validates protocol and
firmware behavior; it does not validate physical CAN electrical margins or RF
certification.
