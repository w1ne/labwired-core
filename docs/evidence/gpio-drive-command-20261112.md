# GPIO drive-command routing evidence

The generic `board.gpio_drive_volts.<pad>` input describes the MCU output-driver command. Each analog model that consumes it must also consume `board.gpio_driven.<pad>`. The existing `board.gpio_pad_volts.<pad>` observation contract is unchanged.

A known disabled or reset-held driver produces an unused zero-volt command with its drive enable false. That value does not describe the external pad voltage. Driver existence is resolved before held-state handling. An enabled driver needs a known output level and a finite positive I/O supply; an unknown driver or unavailable active supply refuses the sample. Standalone co-simulation checks every required command before advancing the analog runner, so an old command cannot survive a missing sample.

The source-bound R3 gate passed 73 tests: 41 routing cases and 32 adjacent machine/domain/World cases, with zero failures or ignores. Cases cover externally driven disabled pads, an output command that differs from the observed pad, unavailable supply, held unknown drivers, per-model command/enable pairing, and no runner advancement after a required command disappears. The companion JSON records exact commands, source hashes, profile, log hashes and elapsed times.

R1 exposed a test-fixture duplicate-peripheral error (34 passes, six failures). Replacing the existing default GPIO fixture produced 72 passing R2 cases without changing the routing source. Independent review then identified the held unknown-driver ordering defect. R3 corrects that ordering and adds its regression. Original failed-run records remain preserved in task evidence.

This gate establishes the command/observation boundary. It does not establish native firmware startup, sensor initialization, commutation, physical voltage accuracy or flight readiness.
