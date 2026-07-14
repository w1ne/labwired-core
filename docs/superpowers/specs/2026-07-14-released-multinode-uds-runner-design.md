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

The environment runner uses the same result schema and output names as the
single-node runner.  Its `result.json` identifies the environment script and
records every assertion; `uart.log` has deterministic node-prefixed console
output; `snapshot.json` is an environment-safe final-state record; and JUnit
reports every assertion.  Configuration errors also yield the standard
machine-readable artifact set.  Unsupported assertion kinds in environment
mode fail explicitly rather than silently weakening a gate.

FDCAN wiring is a small post-construction seam: `World::from_manifest` creates
one `CanBus` per manifest interconnect and attaches an endpoint to each named
node's configured FDCAN peripheral.  The pre-existing `CanBus` continues to
move frames during `World::step_all`; a narrow test proves an attached
peripheral changes from unattached to attached, and a world-level test proves
the manifest rejects an unknown node.

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
