# External Components (ESP32-C6-DevKitC-1)

No required external simulated components for the minimal deterministic smoke.

The onboarding path exposes the on-chip UART0 console only:

1. PCR (`pcr` — native register file; UART0 clock gate is enforced)
2. IO_MUX (`io_mux` — register writes are recorded; pad routing is not
   electrically enforced)
3. UART0 (`uart0` — Espressif UART IP + FIFO, paced at 160 MHz; connected to
   the twin's host capture sink)

The LabWired component-model record maps GPIO16 to `uart0.tx` and GPIO17 to
`uart0.rx`. [Espressif's board guide](REQUIRED_DOCS.md) places the USB-to-UART
bridge on those signals. These pin roles are model claims; the current smoke
observes UART0's host capture, not electrical propagation through the pads or
bridge.

## On-board devices intentionally omitted

The DevKitC-1 carrier includes:

1. **USB-to-UART bridge** — the twin captures UART0 output directly. The
   external bridge and electrical propagation through GPIO16/GPIO17 are not
   modeled.
2. **Addressable RGB LED (WS2812) on GPIO8** — omitted from the system profile.
   It requires an RMT or bit-banged WS2812 frame; a GPIO level does not light it.
3. **BOOT button on GPIO9** — omitted from the system profile. GPIO9 is a
   strapping pin, and the L1 model does not implement download-mode entry.
4. **USB Type-C (native USB Serial/JTAG) port** — the chip descriptor includes
   `USB_DEVICE`, but the board profile does not expose it as a console path.

## Adding an external device (I²C / SPI sensor, etc.)

There is nothing to attach yet on this target: the C6's I2C0 (0x6000_4000),
SPI0/1/2 and GDMA windows are deliberately not declared, so an
`external_devices` entry would have no bus to sit on. Add the controller model
first (see [`docs/peripherals.md`](../../docs/peripherals.md)), then follow the
`external_devices` attach pattern in `examples/demo-blinky/`.
