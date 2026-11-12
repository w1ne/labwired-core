# Compensated Newton linear solve — 2026-11-12

## Result and scope

The generic correction retains the original nonlinear equations, LU factors,
physical circuit, tolerances, Newton iteration budget and timestep policy. It
computes the unfactored residual with error-free products and compensated sums,
then solves up to four corrections using the same factors. If the first
correction fits every existing per-unknown tolerance, it retains the original
solution exactly. This preserves the existing accurate trajectory hashes.

The unchanged complete native input advances through its 100-round, 60-second
probe: 10 microseconds of simulated time in 20.028079 seconds on work-1. All five
MCUs remain held at reset. The environment field `instructions: 500` counts node
advance calls; it is not a retired CPU instruction count. No startup, PWM, motor,
hardware-equivalence or interactive-performance claim follows from this prefix.

## Cause and independent checks

The old solver fails at the current-limit transition. The outer error is
reported at 385 ns; the final retry is at 375.009765625 ns. Diode limiting is
inactive. Large C/h and L/h terms expose a linear-solve roundoff floor that gets
worse as the internal step shrinks. No physical-model or power-sink Jacobian sign
change was needed.

`native-lu-refinement-replay.json` replays two captured 664-unknown systems. It
first reproduces the old Rust LU solution bit for bit for all unknowns. At the
terminal retry, the winding correction is 3.243260e-7 V versus an existing
5.762713e-12 V tolerance. After four corrections, an independent exact-product /
`math.fsum` residual requires at most 1.658818e-5 of the declared tolerance over
all unknowns. This checks consistency with the numerical convergence contract;
it is not a general forward-error theorem for an ill-conditioned system.

The new reduced-circuit regression has separate passive-divider, KCL, winding
DCR, bounded-current and capacitor charge witnesses. The frozen old solver
produces an initial feedback error of 5.602528e-12 V against an unchanged 1e-12 V
zero-state bound. The corrected test preserves that negative control. During
charging, it propagates the existing absolute-plus-relative accuracy of both
nodes through the passive divider.

## Tests and preserved experiments

The corrected native source passed 80 device, 2 linear golden, 47 macro,
1 numerical-accuracy and 10 transient-regression tests: 140 total. Every existing
trajectory hash is unchanged. Formatting and core/CLI all-target Clippy with
warnings denied also passed. The production `mna.rs` hash is identical across
these checks; the new accuracy test was corrected before its final passing run.

The first unconditional refinement experiment changed one existing macro hash
and was rejected. No fixture was re-pinned. The second experiment preserved all
129 existing tests but exposed an overly strict absolute-only assertion in the
new test at 160 ns. Its measured error was within the existing relative accuracy
contract. The corrected new test and all 10 transient tests then passed. Logs
preserve the failed assertion and the final pass separately.

`verification.json` binds these historical native checks to the source hashes.
The fresh generic composition then passed 184 focused tests, including the ADC
and GPIO routes and both upstream/follow-up regulator admission guards. The
normal repository hook execution and final remote head are recorded separately. Native model source and its capture acknowledgment are not added by
this generic correction.

## Files and replay

- `diagnostic-negative-controls.tar.gz` preserves the two unrefined matrix dumps,
  old complete-native failure, reduced-circuit old-solver failure, exact source
  manifest, and original replay script. Its SHA-256 is in `verification.json`.
- `native-prefix/` preserves the exact corrected result, snapshot, analog CSV,
  console and timing receipt. The original world and script hashes are unchanged.
- `native-refinement2.log.gz` preserves the unchanged golden passes and rejected
  new absolute-only assertion. `native-refinement2a-accuracy.log.gz` contains the
  final new oracle and transient passes. `native-refinement2a.log.gz` contains the
  exact prefix, formatting/build and final Clippy result.

Run the Rust oracle through `cargo test -p labwired-core --test
analog_newton_accuracy`. To replay the captured linear systems, extract the
negative-control archive to a temporary directory and run
`python3 replay-native-matrices.py <that-directory>` with NumPy and the Linux
C math library available. This portable wrapper changes only the archived
script's input directory selection.
