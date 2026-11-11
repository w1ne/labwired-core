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

## I²C sensor example

The C6 chip descriptor now declares a behavioral I2C0 controller
(`esp32c3_i2c` model at `0x6000_4000`) and the board pin matrix routes it to
GPIO pads. The runnable
[`ESP32-C6-DevKitC-1-N8 + BME280 example`](../esp32c6-devkitc-1-n8-bme280/)
attaches an Adafruit BME280 model at `0x77` on GPIO6/GPIO7 and verifies its
chip ID and raw temperature. This proves simulated I²C traffic and the sensor
response; it does not prove electrical behavior on a physical bus.
