# External Components — Arduino Nano 33 BLE

None. The smoke firmware exercises the SoC surface only: CPU, memory, UARTE0
and GPIO. The UART is the modelled console and needs no external device — the
engine captures its byte stream directly.

The board carries an onboard IMU and a USB interface; neither is modelled or
attached in this example:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| LSM9DS1 IMU | TWI0, SDA P0.14 / SCL P0.15, addresses 0x6B / 0x1E | not modelled (the TWI0 controller is) |
| USB CDC (`Serial`) | nRF52840 USBD | not modelled — the console is UARTE0 (`Serial1`) |
| Power LED | P1.09 | not modelled (status only) |

When any of these becomes a milestone, it goes here with its fidelity tier and
descriptor path, and the system manifest grows the matching
`external_devices` entry.
