# Selected SAMD IRQ preprocessing diagnostic

Status: selected hosted diagnostic passed in
[run38122098497](https://github.com/CrispStrobe/labwired-core/actions/runs/38122098497)
and merged through [PR196](https://github.com/CrispStrobe/labwired-core/pull/196).
The [unchanged original result and audit boundary](../receipts/2026-10-11-samd-irq-preprocessing/README.md)
bind the exact source, artifact, observed paths and remaining unknowns. The
[interface boundary](samd-critical-section-interface.md) and
[runtime lanes](samd-permissive-runtime-lanes.md) remain authoritative.

## Purpose and frozen inputs

The separate [workflow](../../.github/workflows/samd-irq-preprocess.yml) checks
21 selected C++ translation units: the nine directly observed CODAL core
implementations, the actual ItsyBitsy M4 definition, three SAMD implementations,
six request implementations and the additional consumers of `neopixel.h` and
`melody.h`. This is not all 172 observed units or all 120 header consumers.

[The input manifest](../../scripts/samd-irq-preprocess/input.json) binds four
recorded diagnostic flag groups and 278 unique dependency identities. The
[derivation tool](../../scripts/samd-irq-preprocess/freeze.py) reads only the
already retained original dependency report and generated configuration from
the [original evidence run](https://github.com/CrispStrobe/labwired-core/actions/runs/37794963356).
It verifies their exact original hashes before deriving new public metadata.
It does not acquire evidence, extract firmware or rewrite the originals.

The only path transformations are explicit replacement of the captured source
root by a staging-root placeholder and normalization of redundant include-path
dot segments. Dependency identities are compared as sets: repeated resolved
aliases in the original report are deduplicated. Recipe/flag hashes remain
bound to the originals. The generated
configuration is captured byte-for-byte, not recreated by a substitute generator.
The new commands are separately named diagnostics, not recovered original
invocations or a repetition of the historical firmware build.

## Hosted execution and refusal boundary

Request generation uses the pinned consumer builder in request-only mode, with
its original request hash required. Six immutable public repositories are staged
without recursive submodules, configuration, compilation or linking. Neither
the historical image nor a guest is generated, downloaded, loaded or executed.
The existing original-build workflow is unchanged and is not triggered by these
new script paths.

Before preprocessing, the diagnostic binds the compiler executable and every
selected source/header/toolchain dependency's bytes. It invokes only bounded
`-E` probes, preserving the recorded flags and forced header. Each unit's new
dependency set must equal the frozen separate `-M` observation. A second `-E -dM`
probe records selected effective macro values. Inputs/compiler are checked
again after all probes. Changed inputs or an unbound dependency refuse.

The uploaded result contains source-relative spelling locations, selected
macros and output hashes, not raw vendor-expanded code or host paths. Spelling
observations may include declarations, definitions and string tokens: they are
not established calls or balanced critical sections. Partial progress is retained
as `INCOMPLETE`, never silently promoted after a failure. Pure synthetic tests
cover frozen scope, flag injection, forced-header omission, line markers,
dependency grammar, duplicate JSON, original-byte refusal and hosted-only guards.

## What remains after a successful diagnostic

Review each effective body and macro-expanded caller, including nested timer,
allocator, early-return and terminal-panic behavior. Additional/generated/assembly
callers, actual link retention, notices and execution remain separate unknowns.
Choose an explicit paired-ABI versus saved-token integration policy, then build
a named simulation-only profile without ASF fallback and qualify representative
real callers in clean positive/negative ARM guests. A passing diagnostic does
not admit a runtime, update a consumer pin or establish active PyBadge RTx.
