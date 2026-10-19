# External Components — STM32F4-Discovery

None. The smoke firmware exercises the MCU's own RCC + GPIO + USART2 surface
only; USART2 is the modelled console UART and needs no external device.

The board does carry a LIS302DL/LIS3DSH accelerometer, a CS43L22 audio DAC and
an MP45DT02 MEMS microphone, and the ST-LINK/V2 provides SWD. **None of these
are modelled or attached** in this example — the board manifest lists only the
four user LEDs and the USER button as `board_io`.

When an I²C/SPI/I²S device milestone lands (accelerometer read, audio out),
each device goes here with its fidelity tier and descriptor path, and the
system manifest grows an `external_devices:` entry.
