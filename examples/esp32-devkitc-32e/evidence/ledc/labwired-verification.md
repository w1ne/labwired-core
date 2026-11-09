# Hosted LEDC verification

`labwired_verify` returned `status: model_verified` for the Arduino LEDC
program on the hosted classic ESP32 profile (`board: esp32`). The required
oracle clauses passed:

- `ledc.HSCH0-7_DUTY_R.DUTY_R == 18992` (1187 at 14-bit resolution, in Q4
  register format).
- GPIO23 produced at least two edges. The run captured a rising edge at cycle
  5,479,441 and a falling edge at cycle 5,827,751, a high pulse of about
  1.451 ms at the configured 240 MHz CPU clock.

The run used 9,000,000 steps and returned `run_status: pass`. It reported one
non-blocking model approximation: with no explicit power net in the hosted
diagram, the MCU VDD is assumed to be an ideal 3.3 V rail, so power-on and
brown-out reset behavior are outside this check. This is digital-twin evidence,
not physical-board confirmation. The oracle checks LEDC register state and the
GPIO waveform; it does not assert servo angle or mechanical motion.
