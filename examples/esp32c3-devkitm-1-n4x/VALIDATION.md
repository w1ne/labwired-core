# Validation scope

The committed fast-boot smoke loads a bare RISC-V ELF into the C3 flash window,
captures UART0 output, observes GPIO8 transitions, and requires the attached
NeoPixel model to decode one red pixel. This exercises the C3 GPIO register
model and the WS2812 edge decoder on the exact N4X board profile. Firmware
bit-bangs the GPIO signal; it does not use the C3's minimal RMT model, which
does not model timed symbol playback.

It does not exercise the mask-ROM or second-stage bootloader, the USB-to-UART
bridge, IO_MUX electrical routing, timed RMT playback, Wi-Fi, Bluetooth LE,
buttons, or physical board behavior. No silicon differential was run.
