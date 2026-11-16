# Measured external-device power at analog boundaries

The route `board.device.<device-id>.power.<rail>_volts` supplies an actual circuit voltage to a declared external device. In a World, prefix it with `node.<node-id>.`. It is a numeric model output, not a Boolean enable. Device identity is exact; the parser splits the final `.power.` delimiter, so existing IDs containing dots retain their meaning.

The device must declare `config.supply_rails`. The established device power layer validates its model capability, required rails, and inclusive operating envelopes. Routes do not create rails. Unknown devices or rails, malformed paths, a read direction, duplicate producers, nonfinite/negative values and nonnumeric values are refused. Missing observations retain the previous value; a never-observed rail remains unknown and cannot qualify the device.

At each analog boundary the router validates every present device voltage before publishing any of them. It then submits one batch to the existing SystemBus power API. That API publishes all rails for each device before recomputing its AND gate and generation. Exchanging which of two rails is off therefore cannot create an intermediate power-on/reset event. Invalid batches leave device power untouched.

World construction already primes the analog model and applies its outputs at time zero. This route uses that path, before any CPU instruction. Later updates use the same boundary path as GPIO, reset and motor signals. Power does not require a controller poll. The existing SPI gate resets the actual sensor and parks its interrupt lines at the power transition; transfers and sensor time do not advance while the gate is off.

`MachineTrait::inspect_devices` exposes the existing universal declared-device inspection join for World nodes. It does not pretend a sensor is a display. The BMI270 SPI artifact adds read-only `model_time_ns` so clock parking can be observed both on and off. No protocol behavior changes are introduced by that field.

## Validation and limits

`device_supply_routing` exercises exact IDs, route direction, unknown rails, duplicate producers, invalid-batch rollback, blocked off-state SPI traffic, a real BMI startup delay, time-zero World publication, atomic rail swaps, two-node isolation and sensor-clock parking while CPU and MNA time continue. The fixture uses real STM32F401 nodes with a small Cortex-M loop; it does not execute native flight-controller firmware. The existing device rail, BMI SPI, shared analog and circuit BLDC suites cover the adjacent behavior.

The external-device model records operating availability and reset behavior. It does not model power current, inrush, damage, or sensor accuracy outside its declared envelope. A complete native flight-controller cold-power result still requires the distinct MCU power domains and exact firmware path. The independent bounded native ESC evidence in `docs/evidence/native-esc-four-windings-world-20261112.json` proves only a 10 microsecond operating-point run with explicit demonstration motor parameters, not startup or commutation.
