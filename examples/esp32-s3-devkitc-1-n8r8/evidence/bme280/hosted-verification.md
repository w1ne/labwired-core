# Hosted Arduino verification

Checked 2026-11-10 with LabWired's oracle-gated verifier. The request used the
catalog's generic `esp32-s3-devkitc-1` board model with a BME280 wired to I2C0
GPIO8/GPIO9 and the Arduino sketch shown in the quickstart. The catalog does
not currently expose an exact N8R8 board target.

The first two runs used 1,000,000 and 10,000,000 steps and did not reach the
sketch marker. With a 100,000,000-step budget, the verifier returned
`status: model_verified`; the oracle clause
`serial contains "BME280_ID=0x60"` passed. The captured serial includes the
BME280 ID marker.

The verifier reports a nonblocking approximation: the virtual board does not
route `board.power.vdd_volts`, so VDD is idealized and power-on/brown-out reset
behavior is not simulated. This hosted verdict is for the catalog's generic
DevKitC-1 board model; the local register-level smoke separately exercises the
N8R8-specific 8 MB flash descriptor and BME280 system. No physical-board run
is recorded.
