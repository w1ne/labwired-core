# External Components (ESP32-C6-DevKitC-1)

The on-board addressable WS2812B RGB LED is attached as the `rgb_led` device
and observes GPIO8. The smoke writes a GRB frame to RMT channel 0, whose modeled
TX path routes signal 71 through the GPIO matrix to that pixel. The system also
maps the active-low BOOT switch to GPIO9 while firmware is running.

The onboarding path exercises:

1. PCR (`pcr` — native register file; UART0 clock gate is enforced)
2. IO_MUX (`io_mux` — register writes are recorded; pad function routing is
   not electrically enforced)
3. GPIO (`gpio` — shared C6 register model, GPIO8 pad edges and externally
   driven GPIO9 button input)
4. RMT (`esp32c6_rmt` — channel-0 symbol playback and timed GPIO-matrix output)
5. `rgb_led` (`neopixel` — decodes the WS2812 GRB waveform into a pixel artifact)
6. UART0 (`uart0` — Espressif UART IP + FIFO, paced at 160 MHz; connected to
   the twin's host capture sink)

The LabWired component-model record maps GPIO16 to `uart0.tx` and GPIO17 to
`uart0.rx`. [Espressif's board guide](REQUIRED_DOCS.md) places the USB-to-UART
bridge on those signals. These pin roles are model claims; the smoke observes
UART0's capture sink, not electrical propagation through the pads or bridge.

## On-board devices and behaviors outside this profile

1. **USB-to-UART bridge** — the twin captures UART0 output directly. The
   external bridge and electrical propagation through GPIO16/GPIO17 are not
   modeled.
2. **BOOT button reset behavior** — the GPIO9 contact is modeled while firmware
   is running, but the strapping effect and download-mode entry at reset are
   outside the profile.
3. **USB Type-C (native USB Serial/JTAG) port** — the chip descriptor includes
   `USB_DEVICE`, but the board profile does not expose it as a console path.

## Adding an external device (I²C / SPI sensor, etc.)

The C6 descriptor now declares I²C0, SPI2 and GDMA controller models, but this
board smoke does not connect an external sensor. Attach a supported device to
the controller's pins and add a firmware assertion before claiming that board
path; see [`docs/peripherals.md`](../../docs/peripherals.md) and the
`external_devices` pattern in `examples/demo-blinky/`.
