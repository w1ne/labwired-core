# Known limitations

This onboarding profile covers the Arduino Nano RP2040 Connect's header TX
(RP2040 GP0) and the onboard ST LSM6DSOX accelerometer on I2C0 (GP12/GP13,
Arduino A4/A5). The local smoke checks GP0 edges, sensor identity, the
accelerometer control register, data-ready and one XYZ sample.

The LSM6DSOX model is a partial accelerometer model. Gyroscope, temperature,
FIFO, embedded functions, interrupts, SPI/I3C, electrical behavior, and ODR
timings beyond the tested 416 Hz path are not modeled. The NINA-W102
Wi-Fi/Bluetooth module, MP34DT06JTR microphone, RGB LED, and ATECC608A are also
not modeled. USB and the RP2040 boot ROM are outside the smoke.

The memory map records the board's 16 MiB flash and 264 KiB SRAM, but the
firmware runner loads the image directly after the reserved 256-byte
second-stage bootloader slot. It does not exercise QSPI reads/writes,
physical USB flashing, or board reset behavior. No physical-board capture is
included.
