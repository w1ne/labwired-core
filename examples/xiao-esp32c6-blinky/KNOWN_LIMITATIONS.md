# XIAO ESP32-C6 profile limits

This profile reuses the ESP32-C6 high-performance core and peripheral map. It
does not model or claim:

- Wi-Fi 6, Bluetooth LE, Thread, or Zigbee radio behavior
- the low-power RISC-V core
- USB Serial/JTAG behavior or USB D+/D- electrical behavior
- the RF switch, antenna selection, battery charger, or charging indicator
- reset-time BOOT strap behavior (GPIO9 is only declared as an active-low input)
- board power sequencing, brown-out behavior, or voltage drops; the simulator
  assumes an ideal powered rail for the MCU

The shared C6 GPIO model covers GPIO0 through GPIO25; XIAO's exposed GPIO pins
and GPIO15 LED are within that range. GPIO26 through GPIO30 remain outside the
model. The C6 IO_MUX window records register values but does not enforce pad
function selection electrically. See [`docs/boards/esp32c6-devkitc.md`](../../docs/boards/esp32c6-devkitc.md)
for the shared C6 model's detailed class-by-class boundary.
