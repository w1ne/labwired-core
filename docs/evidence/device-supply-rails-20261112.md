# Atomic external-device supply rails

The existing `SupplyDomain` / `SupplyGate` API now accepts an optional
`config.supply_rails` map. Each rail declares `min_v` and `max_v`; all named
rails must have observed voltages within their inclusive ranges before the
device operates. Unknown rails start off. This is an operating-availability
model; it does not model damage, inrush or electrical loading.

```yaml
external_devices:
  - id: U9
    type: bmi270_spi
    connection: spi0
    config:
      cs_pin: GPIO14
      int1_pin: GPIO13
      supply_rails:
        vdd: {min_v: 1.71, max_v: 3.6}
        vddio: {min_v: 1.2, max_v: 3.6}
```

The map has 1–8 entries. Rail names have 1–64 ASCII letters, digits,
underscores or hyphens. Each entry accepts only the two required finite
numeric fields, with `0 < min_v <= max_v`. The new form cannot be mixed with
legacy `supply_domain` / `supply_min_v`. Static `powered: false` remains off.
Legacy manifests with no routed power retain their existing implicit-power
behavior; they do not constitute a native supply proof.

A SPI model must explicitly implement power transitions and declare every
required rail with its supported operating envelope. A configured range can
be narrower, but cannot omit a required rail, add a fictitious replacement,
or widen the model envelope. BMI270 requires both VDD (1.71–3.6 V) and VDDIO
(1.2–3.6 V), from Bosch BST-BMI270-DS000-08 electrical characteristics. A SPI
controller must expose the existing mutable attached-device capability.
Unsupported device/controller combinations fail attachment.

## Boundary API

`SystemBus::validate_device_supply_route(device, rail)` checks an existing
attachment without creating a rail. `apply_device_supply_voltages` takes a
slice of `(device_id, rail_name, voltage_v)` tuples. It validates the whole
batch before any mutation, refusing unknown devices/rails, duplicate pairs,
negative values and nonfinite values. An empty batch is a no-op. Omitted
rails retain their previous observation.

The method plans all updated device states, then publishes each device's
rails together and recomputes its AND gate once. Swapping which rail is off
cannot create an intermediate on/off cycle. The existing mutable SPI-device
seam publishes power transitions immediately, without CPU execution, elapsed
time or a synthetic SPI transaction. The ordinary pad-drive drain parks a
previously asserted interrupt at that same boundary.

The separate co-simulation router follow-up will map
`board.device.<device-id>.power.<rail>_volts`, with the existing World node
prefix, to this API. This change does not by itself claim that those routes
are already accepted by a released router.

## Model and evidence behavior

The generic SPI gate blocks bus transfers and sample-clock advancement while
off. The model's opt-in callback clears volatile configuration on cutoff and
restore. The BMI implementation clears its actual uploaded image, register
state, sample counters and pending interrupts, then requires the existing
POR/SPI-selection/config-upload sequence again. Unsupported-request audit
history survives software and power resets, so a restart cannot hide earlier
unsupported FIFO or auxiliary traffic.

Artifacts include each declared rail's bounds, observed voltage and
`unknown` / `in_range` / `out_of_range` state, plus the aggregate powered state
and transition generation. Routed SPI runtime snapshots remain explicitly
unsupported. Existing I2C and ESP-AT domains retain their documented register
retention and generation semantics.

## Verification

On 12 November 2026, 54 focused cases passed: ten new rail cases, seventeen
BMI protocol/factory cases, six legacy supply-domain cases, three I2C wrapper
cases and eighteen ESP-AT UART cases. Tests include real configuration RAM
reset, no off-state sampling, immediate GPIO input parking, inclusive bounds,
partial observations, atomic rail swaps, complete-batch rollback and strict
schema/model-capability refusals. Invalid schema variants are exercised
within the named cases. These are model and bus contracts, not physical
captures or an acceptance of the unchanged flight-controller firmware.

The generated peripheral manifest also passes all thirteen registry gates,
including exact fixture equality and the new structural `object` config type.
