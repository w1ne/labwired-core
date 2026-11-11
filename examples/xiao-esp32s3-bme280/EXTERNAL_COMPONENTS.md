# External components and profile scope

The profile attaches one modeled Bosch BME280 at I2C address `0x77` on GPIO5
(SDA) and GPIO6 (SCL), plus the XIAO's built-in active-low LED on GPIO21. The
smoke reads the sensor identification and deterministic raw-temperature
registers and observes LED pad transitions.

The chip descriptor's `system_stub` register window remains zero-answering and
is not accessed by this firmware. The XIAO's physical OPI PSRAM, wireless
radios, USB electrical enumeration, battery circuitry, and other expansion
interfaces are outside this profile. Bus voltage, pull-up resistance,
capacitance, and sensor environmental accuracy are not simulated.
