# External component — ESP32-C3-DevKit-RUST-2

The system connects one `neopixel` model to GPIO2, which Espressif's board
guide and schematic identify as the onboard WS2812B RGB LED data input. The
smoke sends a 24-bit GRB single-wire frame and checks that it decodes as red.

This is a protocol-level model of the data path. It does not model the LED's
power, analog thresholds, brightness/current, or exact die behavior. Firmware
bit-bangs GPIO; RMT is not included in this board slice.
