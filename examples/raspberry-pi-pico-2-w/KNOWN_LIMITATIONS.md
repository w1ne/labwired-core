# Known limitations

This board profile models only the RP2350 GP0 header GPIO path: IO_BANK0 routes
GP0 to SIO, and firmware toggles the SIO output. The smoke asserts at least
eight edges on `sio:0`.

The CYW43439 Wi-Fi/Bluetooth module is omitted, including its firmware,
host interface, radio behavior, and control of the onboard LED. USB device
behavior, UF2 boot and flashing, the second Cortex-M33 core, Hazard3 RISC-V,
PADS_BANK0 electrical behavior, and other RP2350 peripherals are also outside
this profile. This is simulator evidence; no physical-board capture is
included.
