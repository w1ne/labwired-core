# Measured external-device power at analog boundaries

The route `board.device.<device-id>.power.<rail>_volts` supplies an actual circuit voltage to a declared external device. In a World, prefix it with `node.<node-id>.`. It is a numeric model output, not a Boolean enable. Device identity is exact; the parser splits the final `.power.` delimiter, so existing IDs containing dots retain their meaning.

The device must declare `config.supply_rails`. The established device power layer validates its model capability, required rails, and inclusive operating envelopes. Routes do not create rails. Unknown devices or rails, malformed paths, a read direction, duplicate producers, nonfinite/negative values and nonnumeric values are refused. Missing observations retain the previous value; a never-observed rail remains unknown and cannot qualify the device.

At each analog boundary the router validates every present device voltage before publishing any of them. It then submits one batch to the existing SystemBus power API. That API publishes all rails for each device before recomputing its AND gate and generation. Exchanging which of two rails is off therefore cannot create an intermediate power-on/reset event. Invalid batches leave device power untouched.

World construction already primes the analog model and applies its outputs at time zero. This route uses that path, before any CPU instruction. Later updates use the same boundary path as GPIO, reset and motor signals. Power does not require a controller poll. The existing SPI gate resets the actual sensor and parks its interrupt lines at the power transition; transfers and sensor time do not advance while the gate is off.

`MachineTrait::inspect_devices` exposes the existing universal declared-device inspection join for World nodes. It does not pretend a sensor is a display. The BMI270 SPI artifact adds read-only `model_time_ns` so clock parking can be observed both on and off. No protocol behavior changes are introduced by that field.

## Declarative devices

A declarative descriptor opts into routed supply with an optional, bus-agnostic `behavior.power` block (sibling of `behavior.timers`). The SPI engine implements it today:

```yaml
power:
  rails:
    - { name: vdd,   min_v: 1.72, max_v: 3.6 }
    - { name: vddio, min_v: 1.2,  max_v: 3.6 }
  startup_us: 1000
```

Each rail may also carry an optional `pin:` (the package pin that feeds it), which the engine ignores and the diagram compiler reads.

The rails are the required supplies and their operating envelopes (cite the datasheet table next to the block). Without the block the part reports no power-control capability and attaching it with `supply_rails` is refused, as before.

When the gate cuts power the part is dark: `0xFF` on MISO, chip-select edges ignored, no timers, no pin drives. Every power-up is a power-on reset: registers and any register file return to their descriptor reset values, the bus mode returns to its initial state (a part with `activate_on_first_cs_release` is in I2C mode again and needs a new chip-select rising edge), frame state, timers and rule-machine state are cleared, and a `startup_us` interval begins. During start-up the part behaves as not yet answering, so a chip-select edge inside it does not select SPI. Start-up elapses on device time. Sensor stimulus inputs (`SimInput`) are the physical world and survive the cycle. The static `powered: false` config flag is unchanged.

The BMA400 (`configs/devices/bma400.yaml`) declares VDD 1.72-3.6 V, VDDIO 1.2-3.6 V and a 1 ms power-up time from its datasheet. `declarative_spi_supply_por` charges an RC node from an MCU GPIO-switched source, routes the solved node voltage to both rails and checks the answer/no-answer boundaries on one time base.

## Validation and limits

`device_supply_routing` exercises exact IDs, route direction, unknown rails, duplicate producers, invalid-batch rollback, blocked off-state SPI traffic, a real BMI startup delay, time-zero World publication, atomic rail swaps, two-node isolation and sensor-clock parking while CPU and MNA time continue. The fixture uses real STM32F401 nodes with a small Cortex-M loop; it does not execute native flight-controller firmware. The existing device rail, BMI SPI, shared analog and circuit BLDC suites cover the adjacent behavior.

The external-device model records operating availability and reset behavior. It does not model power current, inrush, damage, or sensor accuracy outside its declared envelope. A complete native flight-controller cold-power result still requires the distinct MCU power domains and exact firmware path. The independent bounded native ESC evidence in `docs/evidence/native-esc-four-windings-world-20261112.json` proves only a 10 microsecond operating-point run with explicit demonstration motor parameters, not startup or commutation.
