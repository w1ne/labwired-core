# XIAO nRF54L15 validation

`scripts/onboarding_smoke.sh` builds the Cortex-M33 firmware and runs it against
the narrow `xiao-nrf54l15` system profile. The committed evidence contains the
firmware and CLI build logs, simulator result, GPIO assertion, and generated
onboarding metrics. This firmware does not initialize a serial console because
the board's USB/SAMD11 bridge is outside the modeled profile.

The board profile claims the nRF54L15 reset memory, its three GPIO ports, the
header GPIO map, the active-low user LED, and the active-low user key. It does
not claim USB, UARTE output, radio, ADC, battery measurement, I2C/SPI devices,
the charging circuit, or the Sense-only IMU and microphone. The local run
records `unpowered_rail_assumed`: power-on and brown-out reset are outside the
simulator model. A separate hosted `labwired_verify` attempt was inconclusive
because its generic nRF54L15 board supplied no state evidence for P2.00; see
[`evidence/labwired-verification.md`](evidence/labwired-verification.md). No
hosted `model_verified` claim is made. No physical XIAO board was attached.
