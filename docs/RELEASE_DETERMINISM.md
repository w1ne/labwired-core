# Determinism: Current Guarantees and Release Gaps

## Current execution model

LabWired advances simulated state from the instruction/cycle budget rather than
host wall-clock time. A host pause delays completion in real time but does not by
itself advance simulated time.

For a multi-machine `World`, `World::step_all` currently sorts machine IDs and
steps each machine sequentially before ticking interconnects. This fixed order is
the current synchronization mechanism. Global Virtual Time (GVT), speculative
actors, and distributed snapshots are future work, as stated by the method's
source documentation.

These choices are foundations for repeatability, but they do not by themselves
prove that every observable state is identical on every supported host.

## Floating-point scope

The repository does not currently provide a workspace-wide SoftFloat guarantee.
Do not infer bit-exact cross-host FPU behavior from the deterministic scheduler.
A future guarantee would need an identified implementation plus cross-platform
fixtures that exercise each claimed floating-point instruction path.

## Evidence available today

Determinism and fidelity are supported by narrower, inspectable channels:

- repository tests that run the same model and assert stable outputs;
- committed golden-reference artifacts for selected boards;
- hardware-versus-simulator PC-trace comparisons described in
  [Golden Reference](golden_reference.md);
- per-board validation and the limitations cataloged in
  [`FIDELITY.md`](../FIDELITY.md).

The PC-trace audit establishes PC equality only over its declared exact,
bounded, or prefix scope. It does not currently hash firmware or compare complete
register, memory, UART, or peripheral state.

## Current release gate

`.github/workflows/core-release.yml` builds and packages the CLI and DAP for the
declared platforms, asserts the Linux glibc floor, and starts the Linux artifacts
on supported distribution images before publishing a tag.

The release workflow does **not** currently compare cross-platform VCD hashes
against an `expected_hashes.json` manifest. There is no repository-wide release
gate proving identical VCD, register, memory, and step-count state across every
host. Such a gate remains a future hardening target.

## Requirements for a stronger cross-platform guarantee

Before claiming repository-wide, bit-exact cross-host determinism, a release
gate should:

1. identify a frozen firmware binary and system manifest by digest;
2. run the same cases on every claimed host;
3. define which trace, register, memory, UART, and peripheral fields belong to
   the comparison contract;
4. compare complete declared scopes rather than accepting an undeclared common
   prefix;
5. retain expected artifacts or digests under version control;
6. fail a release when any required observation is absent or differs;
7. document any intentional behavioral change and its hardware justification.

Until that gate exists, state determinism claims at the level of the specific
runner and evidence artifact that measured them.
