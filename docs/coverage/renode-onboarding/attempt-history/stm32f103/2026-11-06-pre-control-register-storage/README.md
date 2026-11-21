# STM32F103 evidence before BME280 control-register storage smoke

Copied from `docs/coverage/renode-onboarding/metrics/stm32f103/` before Round
280 changed the smoke. The archived run was the prior passing measurement
(50,000,000 instructions, 45 JUnit cases, 0 failures), before the additional
`CTRL_HUM` and `CTRL_MEAS` byte-write/read-back checks.
