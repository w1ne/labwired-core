# External components

The system manifest attaches a one-pixel WS2812/NeoPixel observer to GPIO8,
the ESP32-H2-DevKitM-1-N4S onboard addressable RGB LED data pin (J3 pin 8).
The firmware bit-bangs a red GRB frame through the GPIO OUT/ENABLE registers;
the smoke requires the observer to decode that frame. The model does not
represent the physical pad route, signal voltage, current, RMT, or optical
output.

UART0 output is captured directly from the modeled FIFO to the host console.
The board's USB-to-UART bridge and UART pin electrical route are outside this
system profile.
