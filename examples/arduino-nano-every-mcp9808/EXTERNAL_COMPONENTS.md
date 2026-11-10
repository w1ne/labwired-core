# External component treatment

| Part | Connection | Twin behavior | Boundary |
| --- | --- | --- | --- |
| Adafruit MCP9808 breakout, PID 1782 | D20/SDA, D21/SCL, 3V3, GND | Existing declarative MCP9808 model at I²C address `0x18`; register reads are exercised by stock Arduino `Wire` firmware. | Alert output, conversion-time gating, negative temperatures, breakout analog/power behavior, and AVR TWI pad waveforms are not modeled. |

The simulator attaches the sensor to ATmega4809 TWI0 at the controller
boundary. Physical SDA/SCL pin muxing is not part of this profile.
