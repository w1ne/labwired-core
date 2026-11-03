# Known limitations

This onboarding profile covers only the Arduino Nano RP2040 Connect's header
TX pin, which Arduino's pinout maps to RP2040 GP0. Firmware selects SIO in
IO_BANK0 and toggles the output through SIO; the local smoke asserts at least
eight transitions on `sio:0`.

The NINA-W102 Wi-Fi/Bluetooth module, LSM6DSOX IMU, MP34DT06JTR microphone,
RGB LED, and ATECC608A are not modeled. The profile does not claim their bus,
radio, sensor, audio, or cryptographic behavior. USB and the RP2040 boot ROM
are also outside the smoke.

The memory map records the board's 16 MiB flash and 264 KiB SRAM, but the
firmware runner loads the image directly after the reserved 256-byte
second-stage bootloader slot. It does not exercise QSPI reads/writes,
physical USB flashing, or board reset behavior. No physical-board capture is
included.
