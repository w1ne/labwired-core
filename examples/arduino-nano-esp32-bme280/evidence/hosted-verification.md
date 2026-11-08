# Hosted model verification

The committed request and verdict are in
[`verify-request.json`](verify-request.json) and
[`model-verification.json`](model-verification.json). LabWired's mandatory
oracle verifier returned `model_verified`, with both serial clauses passing:

- `BME280_ID=0x60`
- `BME280_TEMP_RAW=0x082DC0`

The hosted diagram identifies the system board as `arduino-nano-esp32`; its MCU
component uses the catalog's `esp32-s3-devkitc-1` S3 model. The wire map uses
the Nano's documented GPIO11/GPIO12 I²C pins. The model reports one
approximation: no power net drives the MCU, so it assumes ideal VDD and does
not model power-on or brown-out reset. This is a digital-twin result, not a
physical board capture.
