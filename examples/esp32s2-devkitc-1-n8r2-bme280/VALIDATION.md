# Validation: ESP32-S2-DevKitC-1-N8R2 + BME280

The committed bare-metal ELF starts directly in the S2 IROM address window. It
configures and checks the GPIO matrix selectors for I2C0 on GPIO8/9, programs
the S2 command-list controller, reads BME280 register `0xD0` and
raw-temperature registers `0xFA..0xFC`, and checks the known model values
before printing its success marker over UART0. The sensor responds through the
configured `i2c0` connection; the transaction is not electrically resolved
through the selected GPIO pads.

The silicon-specific values in this profile come from the ESP32-S2 TRM v1.3:
I2C0 is at PeriBUS2 base `0x60013000`, the controller has 16 commands with
RSTART/WRITE/READ/STOP opcodes `0/1/2/3`, GPIO is at `0x60004000`, the I2CEXT0
GPIO-matrix signals are `29/30`, and I2C_EXT0 is interrupt source 52. The shared
classic Espressif command-list and GPIO models are registered at those S2
addresses. The smoke polls completion, so interrupt delivery is not exercised.

The GPIO model covers GPIO0..39; only GPIO8/9 are used here. Pad-level I2C
electrical resolution, IO_MUX behavior, the remaining S2 GPIOs,
interrupt-matrix behavior, UART pad routing and the CP2102 USB-to-UART bridge
are outside this profile. The model fast-starts an ELF and skips mask-ROM,
flash-MMU, PSRAM, Wi-Fi and USB OTG initialization. The BME280 model provides
a deterministic register vector, not environmental sampling or temperature
compensation.

Evidence in `evidence/` was produced by the repository's `scripts/onboarding_smoke.sh`
runner. The hosted LabWired catalog did not list ESP32-S2 when checked, so this
round has no hosted `labwired_verify` verdict and makes no hardware claim.
