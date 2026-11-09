# Hosted DAC verification

The hosted `labwired_verify` call returned `status: model_verified` with no
gaps. Its register oracle passed both clauses:

- `rtcio.PAD_DAC1.PDAC1_DAC == 128`
- `rtcio.PAD_DAC1.PDAC1_XPD_DAC == 1`

The source called Arduino `dacWrite(25, 128)` on the classic ESP32 profile.
The hosted runner ended with `run_status: incomplete` and
`stop_reason: no_paint` because this DAC-only diagram has no display; the
required DAC register clauses nevertheless passed and LabWired returned its
typed `model_verified` verdict with `gaps: []`.

The source tree includes a focused Rust unit test for the live analog probe:
code 128 should report 1656 mV, clearing XPD_DAC should make the output
undriven, and DAC2 should be independent. That test was not run in this round;
its test-binary compile was stopped after nine minutes without completing. The
local firmware smoke above checks UART output and DAC register state, while the
hosted oracle verifies the same register fields. These are digital-twin results,
not a physical voltage measurement.
