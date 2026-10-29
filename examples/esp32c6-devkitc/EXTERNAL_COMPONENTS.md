# External Components (ESP32-C6-DevKitC-1)

No required external simulated components for the minimal deterministic smoke.

The onboarding path uses on-chip blocks only:

1. PCR (`pcr` — UART0 clock gates; register-backed stub)
2. IO_MUX (`io_mux` — GPIO16/U0TXD pad routing; register-backed stub)
3. UART0 (`uart0` — Espressif UART IP + FIFO, paced at 160 MHz)

## On-board devices intentionally omitted

The DevKitC-1 carrier includes:

1. **USB-to-UART bridge** — the console is modelled as UART0's TX path to the
   capture sink; the external bridge chip itself is not a device in the system.
2. **Addressable RGB LED (WS2812) on GPIO8** — declared as a `board_io` LED pin
   stub. A WS2812 needs an RMT/bit-banged serial frame; a plain GPIO level does
   not light it, and RMT is not on this L1 target. Do not read this stub as an
   RGB output.
3. **BOOT button on GPIO9** — declared as an active-low button stub. GPIO9 is a
   strapping pin; the L1 model does not implement download-mode entry.
4. **USB Type-C (native USB Serial/JTAG) port** — `USB_DEVICE` (0x6000_F000,
   irq 48) is modelled by the shared `UsbSerialJtag` block registered as
   `usb_serial_jtag`. The DevKitC-1's primary console remains the USB-UART
   bridge on UART0; this is the second Type-C / CDC-ACM path.

## Adding an external device (I²C / SPI sensor, etc.)

There is nothing to attach yet on this target: the C6's I2C0 (0x6000_4000),
SPI0/1/2 and GDMA windows are deliberately not declared, so an
`external_devices` entry would have no bus to sit on. Add the controller model
first (see [`docs/peripherals.md`](../../docs/peripherals.md)), then follow the
`external_devices` attach pattern in `examples/demo-blinky/`.
