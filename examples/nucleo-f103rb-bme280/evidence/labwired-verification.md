# Verification boundary — NUCLEO-F103RB + BME280

The committed `onboarding-smoke.yaml` run is local evidence from this
repository's firmware simulator and checks the I2C1/BME280 ID transaction,
USART2 marker, and PA5 LED transitions.

The hosted `labwired_verify` catalog does not list an exact NUCLEO-F103RB
target. No other board was substituted, and no hosted `model_verified` result
is claimed. The local smoke is not physical-board confirmation.
