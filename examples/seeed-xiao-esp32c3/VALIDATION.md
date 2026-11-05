# Validation

On 2026-11-05, the onboarding smoke built a bare-metal RISC-V firmware image,
loaded it into the narrow XIAO ESP32-C3 system, and passed **2/2 assertions**
in 1,000 simulated steps. The GPIO10 edge assertion observed at least eight
transitions on the real ESP32-C3 GPIO model; the second assertion checked the
expected max-step stop.

The committed `evidence/` directory records the build, smoke assertions, and
metrics. This verifies the modeled D10/GPIO10 output path only. It is not a
physical-hardware test or a claim about the board's ROM, Arduino runtime,
wireless, USB, UART, I2C, SPI, ADC, or other functions.
