# Released multi-node UDS runner design

## Context

The public `labwired-test` GitHub Action correctly downloads a released CLI,
but `v0.18.0` cannot execute UDSLib's existing dual-H5 gate.  The gate uses a
declarative `inputs.env` script that creates two Cortex-M nodes and joins their
FDCAN peripherals through a shared `can_bus`.  The capability existed only on
an unmerged Core branch, and its early-return runner did not create the normal
CI artifacts.

The user-facing contract remains intentionally small: build the project's
firmware, invoke the pinned public Action with a YAML script, and consume its
report.  Consumer workflows must not clone or compile LabWired Core.

## Options considered

1. Keep the old UDS workflow's Core clone and source build.  This runs the
   historical feature branch but violates the released-runner contract.
2. Teach the Action to download an arbitrary branch build.  This hides source
   coupling behind an Action and is neither reproducible nor a release.
3. Restore the native environment-test capability on current Core, give it the
   ordinary artifact lifecycle, and publish a new immutable release.  This is
   the selected design.

## Architecture

`labwired test` accepts two disjoint v1.x input forms:

- `inputs.firmware` with an optional `inputs.system` continues through the
  existing single-machine runner unchanged.
- `inputs.env` selects a new environment-runner path.  It resolves the
  environment manifest relative to the script, builds a `World`, attaches
  declared `can_bus` endpoints to named FDCAN peripherals, steps all nodes,
  and evaluates node-qualified memory assertions.

The environment runner uses the same outer result contract and output names as
the single-node runner.  Its `result.json` identifies the environment script
and records every assertion; `uart.log` records deterministic node-labelled
console output; `snapshot.json` is an environment-safe final-state record; and
JUnit reports every assertion.  Configuration errors also yield the standard
machine-readable artifact set.  Unsupported assertion kinds in environment
mode fail explicitly rather than silently weakening a gate.

FDCAN wiring is a small post-construction seam: `World::from_manifest` creates
one `CanBus` per manifest interconnect and attaches an endpoint to each named
node's configured FDCAN peripheral.  Every world node is built with
`configure_cortex_m`, matching the normal CLI path rather than constructing a
bare CPU.  Nodes are stepped and rendered in stable id order.  The
pre-existing `CanBus` continues to move frames during `World::step_all`; a
narrow test proves an attached peripheral changes from unattached to attached,
and a world-level test proves a receiver gets a frame and the manifest rejects
an unknown node.  The UDS acceptance assertion remains tester-to-ECU behavior,
not a self-echo claim.

Each `can_bus` must provide a nonblank string at `config.peripheral`; there is
no implicit `fdcan1` default. It must name at least two distinct manifest
nodes. World validates a copied, lexically sorted membership list and attaches
endpoints in that same order, so permuting YAML `nodes:` membership cannot
change the delivery order of simultaneous frames. Unknown nodes and missing or
non-FDCAN peripherals fail with topology-specific diagnostics.

## Explicit environment-runner contract

`schema_version: "1.0"` accepts exactly one input shape.  A script has either
`inputs.firmware` (single machine) or `inputs.env` (world); it cannot contain
both.  `inputs.env` is resolved relative to the test script, and every
node/system/firmware reference in that manifest is resolved relative to the
environment manifest.  CLI `--firmware` and `--system` overrides are rejected
for an environment script because they would make the topology ambiguous.

The first released environment mode accepts only `memory_value` assertions and
each one must name an existing `node`.  It rejects UART, regex, stop-reason,
UDS-tester, fault, verdict, stimulus, VCD, and no-progress-only features with
an `error` result and a diagnostic instead of silently ignoring them.  A memory
value uses the existing little-endian 1/2/4-byte or 8/16/32-bit size rules.

`max_steps` means world rounds: in one round every node executes exactly one
step in stable lexical node-id order, then every interconnect ticks once.
`max_cycles` is the greatest final node cycle count.  `max_uart_bytes` is the
sum of all captured node UART bytes, and `wall_time_ms` covers the whole world
run.  Reaching a configured limit is `max_steps`, `max_cycles`,
`max_uart_bytes`, or `wall_time` in `result.json`; assertions still determine
the pass/fail verdict as in the single-node runner.

Environment output is intentionally unambiguous:

- `result.json` retains `status`, stop reason, metrics, and assertion results,
  and names `config.environment` rather than pretending there is one firmware.
- `uart.log` contains sorted node sections headed `[node:<id>]`; it does not
  silently interleave a hash-map-dependent byte stream.
- `snapshot.json` has `type: "environment"` and sorted per-node cycle/final
  state records.
- `junit.xml` has one run case plus one case per assertion, matching the
  single-node failure/error semantics.

The public Action source is pinned by a full commit SHA; the `version` input is
an immutable Core release tag.  The release pipeline runs a multi-node YAML
with the built archive and the published OCI image before either artifact is
considered usable.  UDSLib's consumer contract bans Core clones, Cargo builds,
and descriptor copying in its workflow.

## Release and migration

Core publishes this support as `v0.19.0`, with release archives and the GHCR
runner image.  The public composite Action's default and examples move to that
release; its source remains pinned by a full commit SHA in consumer workflows.

Only after the release smoke passes will UDSLib change its nightly dual-H5
workflow to `version: v0.19.0`.  The existing gate YAML, its 27-service
memory oracle, and all firmware builds stay intact.  The real dispatched UDS
run is the acceptance test for the restored decoder/FDCAN behavior; no old
decoder commit is copied unless that evidence exposes a current-Core defect.

## Validation

- Config unit tests prove `inputs.env` selects the environment script and
  preserves single-node parsing.
- Core tests prove post-build FDCAN attachment and `can_bus` manifest errors.
- CLI integration tests exercise environment success, assertion failure, and
  configuration errors with `result.json`, `uart.log`, snapshot, and JUnit
  output.
- The release workflow smoke checks the released archive and OCI image.
- UDSLib builds both H5 ELFs and invokes the published v0.19.0 Action; the
  resulting report must show the authentic `SERVICES 27/27 PASS` transcript
  and passing 27-service memory oracle.
