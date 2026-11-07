# Known limitations

This profile includes only the Cortex-M33 RP2350 execution path, UART0 output
for simulator evidence, I²C0, the GP4/GP5 route, and the external BME280 model.

The CYW43439 Wi-Fi/Bluetooth module is omitted, as is its host interface and
control of the onboard LED. USB boot/CDC, physical UART routing, RP2350
Hazard3/RISC-V execution, PADS_BANK0 electrical behavior, board power behavior,
and physical-board parity are outside the profile. The radio is absent from the
descriptor; it is not represented by a zero-answering stub.
