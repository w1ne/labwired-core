# LiteX Ibex before the litex_timer model

Measurement preserved before the chip descriptor was corrected. `timer0` was
declared as `litex_timer_csr32`, which is not a factory type: the fuzzy
`contains("timer")` rule in `canonical_peripheral_type` resolved it to the
STM32 general-purpose `timer` model. The smoke passed only because its
`ev_pending` write (meant as W1C) landed in STM32 CCMR1 at +0x18, which stores
and reads back the 1, so the "wait for zero event" loop exited without any
countdown.
