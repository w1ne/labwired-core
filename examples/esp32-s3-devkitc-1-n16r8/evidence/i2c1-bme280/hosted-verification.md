# Hosted verifier result

The LabWired hosted verifier was run on 2026-11-10 with the Arduino
`Wire1.begin(41, 42, 100000)` BME280 example and an explicit serial oracle for
`BME280_ID=0x60`.

- Status: **failed**; the oracle did not match.
- Captured application output: `BME280 not found on I2C1`.
- Stop reason: the 100,000,000-step budget was exhausted.
- The hosted board description lists I²C0 as its default bus and omits I²C1
  from the bus summary. The hosted run did not attach the BME280 to I²C1, so
  this result does not verify the second-controller Arduino path.

The local repository smoke is separate: it executes the register-level I²C1
firmware against the local ESP32-S3 model and BME280 device model, and passes
the ID and raw-temperature assertions. Treat this as local model smoke evidence;
there is no hosted `model_verified` result for I²C1.
