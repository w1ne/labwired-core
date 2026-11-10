# External component — ESP32-C3-DevKitM-1-N4X

The system attaches one `neopixel` component to GPIO8, the onboard addressable
RGB LED identified in Espressif's board guide. The schematic calls the LED
`SK68XXMINI-HS` (D6); the smoke sends a 24-bit GRB single-wire frame and checks
the decoded red pixel.

This is a protocol-level model of the board's data path. It does not model the
LED's power, analog thresholds, brightness/current, or exact die behavior. The
firmware writes timed symbols to C3 RMT TX0 and routes the output through the
GPIO matrix. The LED model validates the edge timing and decoded color; it does
not model analog or optical behavior.
