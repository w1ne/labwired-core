# IO-Link Multi-Port Demo

This demo models an IFM-inspired multi-port IO-Link master station.

Native mode uses the real `iolinki-master` stack behind the LabWired bridge.
The default browser-safe path may use the hand-rolled fallback until native
IO-Link support is available in that environment.

This is simulation evidence, not IO-Link electrical conformance evidence.

## Analog: L+ supply monitor and NTC

`analog-lplus-ntc-system.yaml` solves two generic netlists on this board's
STM32L476 ADC1 pad map:

- PC0 (ADC1_IN1): the 24 V IO-Link L+ rail through a 100k / 12k divider, driven
  by `ui.iolink.l_plus_volts` (18 V to 30 V keeps PC0 under 3.3 V).
- PC1 (ADC1_IN2): the Beta NTC divider, driven by `ui.ntc.temperature_c`.

```bash
cargo test -p labwired-core --test analog iolink_multiport_demo_monitors_lplus_and_ntc_through_adc1
```

The test sweeps L+ at 18, 24 and 30 V and asserts the solved PC0/PC1 voltages
and the 12-bit `ADC1_DR` codes (24 V reads 3190). This is model evidence, not
silicon evidence.
