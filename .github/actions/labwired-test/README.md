# Core labwired-test action

This directory contains the public Core action used by release smoke tests and
consumer workflows. It downloads a pinned Core release archive rather than
compiling Rust on the consumer's runner.

For a copy/paste consumer workflow pinned to an immutable action-source commit,
use the [CI integration guide](../../../docs/ci_integration.md). This README
intentionally documents the action beside its implementation rather than
choosing its own source SHA: every immutable revision therefore carries its own
matching input and report contract.

The `version` input defaults to `v0.19.1` and independently selects the
immutable Core CLI release archive named
`labwired-v0.19.1-<platform>.tar.gz`. The action downloads that public archive
from the `w1ne/labwired-core` GitHub release with `curl`; it does not build Core
on the consumer runner.

Its inputs are exactly `script` (required), `version` (default `v0.19.1`),
`output-dir`, and `args`. `args` is whitespace-separated extra CLI flags; shell
quoting inside this input is not interpreted.

Every invocation passes `--junit output-dir/junit.xml`, writes `summary.md` and
a self-contained `report.html` into `output-dir`, appends the Markdown report to
the GitHub job summary, and always uploads the entire output directory as an
artifact, including when the test fails.

Use the step outputs when a workflow needs to surface or link the generated
evidence: `status`, `summary-md`, `report-html`, `artifact-url`, and `exit-code`.
