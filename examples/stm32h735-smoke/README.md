# STM32H735 smoke and analog routing

The `system.yaml` profile runs the H735 tier-1 smoke fixture. The separate
[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) fixture routes a
generic Beta NTC divider and an unrelated Shockley diode load line through
PA0/ADC1_INP16 and PA1/ADC1_INP17.

Run the board-specific analog proof from `core/`:

```bash
cargo test -p labwired-core --test cosim_stm32h735_analog \
  stm32h735_smoke_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end -- --nocapture
```

The test checks the solved voltages, 12-bit ADC1 conversion results, and that
channel 18 remains undriven. This is simulator-model coverage, not silicon
validation.
