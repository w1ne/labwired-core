# Hosted catalog capability check

Checked on 2026-11-10. The hosted labwired_describe lookup resolves
esp32-s3-devkitc-1-n16r8 as runnable and lists I²C0 as its default bus. Its
component lookup for sht40 returned NOT_FOUND, with SHT30, AHT20 and SHT31 as
near-name suggestions. No hosted SHT40 run or model_verified claim is
available for this extension.

The committed local smoke is independent: it runs the bare-metal binary on the
repository's ESP32-S3 model, attaches the repository's SHT40 model to I²C1, and
checks the serial response, measurement bytes, and CRCs. This is local model
smoke evidence, not a hosted or physical-board result.
