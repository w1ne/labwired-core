# Validation

The committed onboarding smoke builds the local firmware crate and executes
it against the repository's `nucleo-f767zi-lsm6dsox` system descriptor. The
assertions require the sensor identity, configured data-ready accelerometer
sample, USART3 pass line, and LD1 PB0 edge. The script's stage logs and
simulation result are in [`evidence/`](evidence/).

This is a local deterministic CLI simulation result. The hosted
`labwired_verify` board catalog does not currently expose STM32F767ZI, and no
physical board/sensor capture was made in this round.
