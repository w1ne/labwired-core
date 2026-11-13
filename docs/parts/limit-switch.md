# SPDT limit switch

`limit-switch` is a genuine passive changeover contact with **COM**, **NO**
and **NC** terminals. `pressed=0` connects COM to NC; `pressed=1` connects COM
to NO. Both contact outputs share one actuator and one public stimulus.

```yaml
external_devices:
  - id: upper
    type: limit-switch
    connection: gpio
    config: {nc_pin: PD2, common_high: false}
```

With COM→GND, NC→Uno D2 and `INPUT_PULLUP`, the released switch reads LOW;
pressing it opens NC and the input reads HIGH. To use NO instead, configure
`no_pin`; it reads HIGH released and LOW pressed. Both contacts may be wired
to different MCU inputs. `common_high: true` represents COM→VCC with pulled
down inputs and reverses these levels.

The engine reuses the passive contact model. The open level stands for the
external pull resistor; it is not a simulated mechanical or electrical output
driver. Contact resistance, bounce, travel and automatic coupling to a desk's
shaft/height are outside the model. Stimuli must explicitly actuate the limit.

`desk_controller_inputs` tests NO, NC, both COM polarities, simultaneous
changeover and real Arduino `digitalRead` behavior. This proves the software
model's contact contract, not a physical switch measurement.

Contact arrangement reference: [Omron SS series subminiature basic switches](https://omronfs.omron.com/en_US/ecb/products/pdf/en-ss.pdf).
