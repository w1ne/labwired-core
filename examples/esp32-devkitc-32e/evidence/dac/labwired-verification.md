# Hosted DAC verification

The hosted `labwired_verify` call returned `status: model_verified` with no
gaps. Its register oracle passed both clauses:

- `rtcio.PAD_DAC1.PDAC1_DAC == 128`
- `rtcio.PAD_DAC1.PDAC1_XPD_DAC == 1`

The source called Arduino `dacWrite(25, 128)` on the classic ESP32 profile.
The hosted runner ended with `run_status: incomplete` and
`stop_reason: no_paint` because this DAC-only diagram has no display; the
required DAC register clauses nevertheless passed and LabWired returned its
typed `model_verified` verdict with `gaps: []`. The local peripheral test also
checks the live analog probe reports 1656 mV for code 128 and becomes undriven
when XPD_DAC is cleared. This is digital-twin evidence, not a physical voltage
measurement.
