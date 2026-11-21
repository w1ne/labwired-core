# IO-Link Multi-Port Demo

This demo models an IFM-inspired multi-port IO-Link master station.

Native mode uses the real `iolinki-master` stack behind the LabWired bridge.
The default browser-safe path may use the hand-rolled fallback until native
IO-Link support is available in that environment.

This is simulation evidence, not IO-Link electrical conformance evidence.

The system profile also supports a generic mixed-signal co-simulation proof:
the Beta NTC divider and independent Shockley diode from
`examples/nucleo-l476rg/analog-ntc-diode-adc.yaml` resolve to modeled ADC1
channels 5 and 6 while the four-port IO-Link device remains present. The
register-level ADC conversion is tested by
`cosim_nucleo_l476_analog::iolink_multiport_demo_ntc_and_diode_convert_on_adc1_beside_iolink_device`.
This profile has no pad-to-ADC alias map, so the test makes no PAx or silicon
conversion claim.
