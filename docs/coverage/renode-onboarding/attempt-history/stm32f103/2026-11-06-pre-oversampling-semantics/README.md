# STM32F103 before BME280 oversampling semantics

Archived the prior passing imported-system run before replacing its metrics.
It booted the STM32F103 profile and passed 49/49 smoke assertions, including
live BME280 temperature, humidity, and pressure inputs plus control-register
storage. At this point the BME280 returned its driven raw values even when
oversampling was disabled, and CTRL_HUM did not latch on CTRL_MEAS writes.

The run did not claim conversion timing, operating-mode transitions, filtering,
or status transitions.
