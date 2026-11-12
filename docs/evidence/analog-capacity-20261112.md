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

## Review dependencies

This branch is stacked on the existing World power change
`42a82cfc6ce270cdffb2ac7b2b54b99f402d9a05` (PR1835,
`fix/world-analog-power-20261112`). It includes the same two-method Wasm
inspection error-boundary correction as
`9cdadc09a49d69b621059e20477d0cc68f550e56` (PR1833); its local cherry-pick is
`f2b4942a6`. These dependencies remain separately reviewed changes. The
capacity option does not introduce a second World or Wasm execution path.

## Native circuit follow-up

The separate native ESC electrical deck has 379 unknowns and 528 elements,
SHA-256 `40b1745f2371b54431bd161cad10f925dc64aa6cc65391a062e9fb5470f286f6`.
Its unchanged affine-descriptor export remains the base. A separately hashed
model overlay changes only `config.max_unknowns` to 512:
`b0f003d317958aa416f2365fa89730040ef10700cc12051e2605769adb4a3869`.

The combined production CLI, SHA-256
`6fc6e4e668abdb12f6995b9e3a962e92599b12054af31a37550c21ac35335b8e`,
ran the unchanged deck with all 80 seeds. Its default run exited with the
expected 379 > 128 admission error. The explicit-512 run completed 100 steps
of 100 ns, with 96 trace channels and 101 samples including the initial state.
The trace SHA-256 is
`e5739b6f9556da41f3399b375794e1b56d92327d753e72db10c43c1c43496c27`.
The recorded wall time was 3.82 seconds under concurrent workspace load.

[Exact run receipt](analog-capacity-20261112/native-production-receipt.json)
and [compressed trace](analog-capacity-20261112/native-production-trace.csv.gz)
preserve the command inputs, immutable runner identity and output hashes.
The combined source contains separately reviewed signed-switch and World
changes as well as pending MCU onboarding; this receipt is not a claim that
those dependencies are merged or released.

All 40 voltage drives were held at zero and all 40 drive-enable controls were
false. This 10-microsecond run establishes capacity and convergence only. It
does not execute a CPU, reproduce a cold power ramp, or establish native
startup, PWM behavior, motor motion or flight readiness.
