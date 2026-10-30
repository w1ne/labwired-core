# External Components — MIMXRT595-EVK

None for the FLEXCOMM USART+LED L1 slice. The smoke exercises the MCU's own
CLKCTL/RSTCTL + IOPCTL + GPIO + FLEXCOMM0 surface; FLEXCOMM0 is the modelled
console and the on-board LPC-Link2 VCOM bridge is not modelled.

The MIMXRT595-EVK carries further parts, for example the Octal-SPI NOR flash
(MX25UM51345G) and PSRAM, the audio codec, the digital microphones, the
accelerometer, the USB and SD connectors and the display/camera headers.
**None of these are modelled or attached** in this example. The system
manifest lists only the RGB LED (P0_14, P1_0, P3_17) and SW1/SW2 (P0_25,
P0_10) as `board_io`.

When a later slice attaches one of them, it goes here with its fidelity tier
and descriptor path, and the system manifest grows an `external_devices:`
entry.
