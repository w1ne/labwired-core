# Controlled internal averaged BUCK stages

This feature binds a descriptor-declared internal regulator to the existing in-core adjustable `BUCK` model. It does not add a second electrical solver. The actual VIN, analog-enable supply, SW, feedback and ground nodes remain part of the circuit. External inductors, their DCR, capacitors and loads remain explicit MNA elements.

## Descriptor and ownership

`ChipDescriptor.internal_regulators` is optional. If present it declares 1–16 stages. Each has a distinct 1–64 character ASCII alphanumeric/underscore/hyphen `id`, `kind: buck`, one exact `control_peripheral`, `input_domain`, `enable_domain`, physical pin groups `switch_output`, `feedback`, `ground`, an `initial` configuration and explicit `model` assumptions. Each physical role has 1–64 distinct string labels. One controller owns one stage. VIN and enable can share a circuit net; SW, feedback and ground remain distinct functional roles.

The initial configuration contains `enabled`, `target_voltage_v`, `current_limit_a` and `rated_load_a`. The model contains `max_duty`, `quiescent_current_a`, `output_resistance_ohms`, `efficiency` and `open_loop_gain`. These parameters must be explicit, finite and valid. Target voltage is allowed to exceed VIN so real dropout remains observable. Rated load is independent of the current-limit setting.

A stage requires one complete inline analog model owner before execution. CLI/standalone, Wasm and World admission reject a declared stage with no model. The authoritative Machine advance path also refuses an unbound stage. Legacy chips without internal stages retain their existing empty-model behavior. Runtime snapshots and fixture-epoch replacement do not restore the stage's coupled electrical/command history and are refused for these nodes.

`config.regulators` is a map of 1–16 total stages per analog model, keyed by a 1–64 character ASCII alphanumeric/underscore/hyphen port prefix. Each value names an existing adjustable BUCK X instance, its real enable node, positive finite VIN/enable operating envelopes, and a canonical initial configuration with revision zero. Multiple MCUs can share this one model. The MCU descriptor, initial BUCK VREF/ILIM, model-card assumptions and sampled domain nodes must agree. Generated primitives of a controlled macro cannot acquire another source or thermistor owner.

## Boundary control contract

The MCU-to-circuit inputs under `board.regulator.<id>.` are `enabled`, `target_voltage_v`, `current_limit_a`, `rated_load_a`, and `command_revision`. The seven circuit-to-MCU observations are `input_voltage_v`, `feedback_voltage_v`, `input_current_a`, `output_current_a`, `output_enabled`, `qualified`, and `applied_revision`. World adds the ordinary `node.<id>.` prefix. One complete bundle belongs to one model and one positive sample period.

Revisions are integers from zero through 9007199254740991. Every state-changing accepted command gets a new revision. At most 32 ordered requests can remain pending. A circuit applies one exported frame over its next interval. A result acknowledges only the revision actually exported for that interval; it cannot acknowledge a future request issued while the interval ran. A reset clears queued commands, requests the declared initial configuration with a new revision, and retains physical observations and the identity of an in-flight older command.

World holds inputs at interval start. Standalone sessions also hold regulator commands from the owning model's interval start, including when an unrelated faster model steps. Other legacy standalone input coupling is unchanged. Every owning output boundary requires all seven fresh observation fields. Partial, duplicate, reversed, skipped, nonfinite and mistyped frames fail before observation publication. Models with a different rate do not advance this stage's observation clock.

The generic `Peripheral::attach_regulator_control` capability defaults to refusal. `sync_regulator_control` runs after an atomic observation batch on each affected controller, including t=0 before CPU transactions. A controller can therefore update feedback hysteresis on every accepted sample without depending on firmware register reads. Reset callbacks do not discard solved power observations.

## Electrical behavior and limits

The solver qualifies real VIN and enable voltages during Newton iteration, including the initial operating point. An absent or out-of-envelope supply cannot energize one initial interval. Command enable is ANDed with both supply envelopes. HIZ removes the owned output conductance, input power sink and quiescent-current path. It does not reset the external reactive state or replace the feedback node with a fixed supply.

For supported forward operation, the existing port law is `P_in = P_out / efficiency + P_quiescent`. Positive output current flows from SW toward the external circuit. The current-limit clamp remains independent of the rated-load diagnostic. No rating-based clamp or hardware protection is invented.

`qualified` means only that VIN and the enable supply are within the declared operating envelopes. It does not mean full device qualification. Trace-only `rated_load_a` and `load_within_rating` channels report the applied frame's rating and `abs(output_current_a) <= rated_load_a`. They accompany the actual signed current and `applied_revision`. This absolute comparison does not qualify reverse operation.

The inherited averaged power law and one-sided current limiter do not model active reverse current. A controlled stage refuses reverse output or input current and negative solved output, input or quiescent power at its operating point or an integrated substep. The explicit 1 pA and 1 pW allowances cover numerical residuals only. Positive output current at a negative SW voltage also fails the power check. No reverse-flow result is accepted as an MCU observation. HIZ remains an ideal open output path; unmodeled body diodes, leakage and parasitic switching energy are not added implicitly.

This is an averaged functional controller. It does not establish switching ripple, silicon compensation, efficiency curves, startup-current transition timing, thermal behavior, parasitic discharge paths or physical performance. Exact chip adapters must identify any assumptions used to select model parameters or mode transitions. A firmware run must retain the selected coupling period and its sampled-edge limits in the receipt.

## Validation receipts

The [focused fixture receipt](evidence/controlled-internal-buck-20261112.json) records exact source hashes, test cases, result counts and build profile. The [independent source review](evidence/controlled-internal-buck-review-20261112.json) covers admission, ownership, interval holding and forward-power scope. These receipts describe generic synthetic tests. They are separate from native-board execution and hardware capture acknowledgment.
