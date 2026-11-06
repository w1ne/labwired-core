# Analog joystick

`joystick` is two potentiometers and an independent normally-open SW contact.
Public channels are `x` and `y` in **−1…+1**, centered at **0**, and `pressed`
in 0…1. They all target the placed component id, for example `joy`; callers
never need generated names such as `joy:y` or the potentiometer's internal
`position` channel.

```yaml
external_devices:
  - id: joy
    type: joystick
    connection: adc
    config:
      x_channel: 2
      y_channel: 0
      sw_pin: PD4
      supply_mv: 5000
```

This Uno wiring is VRX→A2, VRY→A0, SW→D4, VCC→5V and GND→GND.
Use `INPUT_PULLUP` for SW. An axis at −1 produces 0 V, at 0 produces VCC/2,
and at +1 produces VCC. Axes reuse the declarative potentiometer model.
Omit any unwired axis/channel or `sw_pin`: input discovery then omits that
unconnected output. `x_connection` and `y_connection` optionally choose
different ADC controllers; otherwise both use `connection`.

The model supplies static, deterministic positions and a clean push-contact
transition. It does not model spring motion, wiper noise or contact bounce.
The `desk_controller_inputs` integration tests execute real Arduino ADC and
digital-input reads and prove X does not change Y.
