# External Components — NUCLEO-L552ZE-Q

None. The smoke firmware exercises the MCU's own RCC + GPIO + LPUART1 surface
only; LPUART1 is the modelled console UART and needs no external device.

The board carries the ST-LINK/V2-1 (SWD + Virtual COM Port), Arduino/morpho
headers, and the usual user LEDs/button. **None of the external headers or the
USB bridge are modelled** in this example — the system manifest lists only the
three user LEDs and the B1 button as `board_io`, and the console is the
LPUART1 byte stream, not a USB CDC device.

When a SPI/I²C device milestone lands (external flash, sensor), each device
goes here with its fidelity tier and descriptor path, and the system manifest
grows an `external_devices:` entry.
