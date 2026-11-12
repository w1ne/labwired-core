# Bosch BMI270 configuration fixtures

These are byte-preserving extracts of the two configuration arrays in the official Bosch Sensortec BMI270 Sensor API at commit `41129fcfe39c583ee5462d79195741945d51c1fe`. They retain the BSD-3-Clause notice in LICENSE-BOSCH.txt.

- `base.bin`: `bmi270.c`, `bmi270_config_file`, 8192 bytes; SHA256 `2d75e68e343a13ff99be98261dfbd99d9e8c6f267da9e3c5aeb883276ad178db`.
- `maximum-fifo.bin`: `bmi270_maximum_fifo.c`, `bmi270_maximum_fifo_config_file`, 328 bytes; SHA256 `e17bf316de8139afa8f9b27cd3520845c7809fb418cdd3f8439dcdcdea683ffe`.

Source: <https://github.com/BoschSensortec/BMI270_SensorAPI/tree/41129fcfe39c583ee5462d79195741945d51c1fe>.

The SPI model verifies these image bytes before reporting initialization success. It does not execute the feature-engine microcode. Recognition of the maximum-FIFO image does not imply a FIFO simulation. Tests use the actual manufacturer arrays, not invented payloads accepted by a byte-count-only check.
