# Failed H753 ADC fixture location retry

The second target-only run changed the smoke schema version but retained the
fixture at the wrong document level. It therefore stopped on the same unknown
`external_devices` field before booting. This retry's logs, metrics, simulator
output, and input script are preserved here. The fixture is now being moved to
`configs/systems/onboarding/stm32h753.yaml`, matching the working H743 setup.
