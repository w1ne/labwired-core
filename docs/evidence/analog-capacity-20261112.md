# Explicit bounded analog capacity

The analog model accepts optional `config.max_unknowns`, an integer from 1
through 512. The default remains 128. `Solver::new` retains the old default;
`Solver::new_with_max_unknowns` is the explicit standalone entry point.
Both constructors guard the limit and circuit size before allocating dense
matrices. No solver equations, integration order or device model changed.

Two matrix buffers consume 256 KiB at 128 unknowns or 4 MiB at 512. Dense LU
remains cubic in matrix size. This option is bounded admission, not an
unbounded allocation policy or a performance guarantee for long firmware
runs. A circuit above its selected limit remains an error.

## Verification

On 12 November 2026, 45 core cases passed: the new capacity contracts plus
existing solver, adapter, linear golden and extracted-copper regressions.
The explicit 379- and 512-unknown ladders match their analytic voltages.
Default 129, explicit 513 and invalid configured limits are refused.
Malformed limits include fractions, floating-point integer notation,
nonfinite numbers, strings, null, booleans and compound values. Existing RC
transients retain bit-identical outputs in both integration modes.

The real manifest validator and adapter factory agree. A shared World with
one existing STM32 fixture builds only within the selected bound; this test
adds no MCU behavior assumption. The production CLI `cosim-step` and actual
Wasm constructor boundary each passed their admission/refusal test. These
47 cases use the same core parser and bounded constructor.

## Native circuit follow-up

The separate native ESC electrical deck has 379 unknowns and 528 elements,
SHA-256 `40b1745f2371b54431bd161cad10f925dc64aa6cc65391a062e9fb5470f286f6`.
Its unchanged affine-descriptor export remains the base. A separately hashed
model overlay changes only `config.max_unknowns` to 512:
`b0f003d317958aa416f2365fa89730040ef10700cc12051e2605769adb4a3869`.

The production CLI follow-up must use a combined core with the already
reviewed signed-switch change and shared-world dependencies. It is limited
to 100 steps of 100 ns with 40 voltage drives held at zero and 40 drive-enable
controls false. Passing it demonstrates bounded capacity and convergence;
it does not demonstrate native MCU startup, a cold power ramp, PWM behavior,
motor motion or flight readiness. No such native result is asserted here.
