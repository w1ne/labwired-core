# Hosted model verification

The committed request and result are in [verify-request.json](verify-request.json) and [model-verification.json](model-verification.json). LabWired's oracle verifier returned `model_verified` for the exact Arduino sketch at [src/main.ino](../src/main.ino), with these clauses passing:

- `INA219_CONFIG=0x399F`
- `INA219_VBUS_MV=12000 I_MA=250 P_MW=3000` after the simulated INA219 inputs were set to 12 V and 0.25 A.

The hosted diagram uses the Arduino Nano ESP32 board identity, the ESP32-S3 MCU model, GPIO11/GPIO12 for I²C, and the INA219 component. The verifier reports one approximation: no power net drives the MCU, so it assumes ideal VDD and does not model power-on or brown-out reset. The sketch is intentionally long-running; verification stopped at its step budget after satisfying the oracle. This is a model result, not a physical-board capture.
