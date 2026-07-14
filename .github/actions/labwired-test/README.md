# Core labwired-test action

This directory contains the public Core action used by release smoke tests and
consumer workflows. It downloads the pinned Core release archive rather than
compiling Rust on the consumer's runner.

~~~yaml
- id: labwired
  name: Run LabWired tests
  uses: w1ne/labwired-core/.github/actions/labwired-test@c6f8c68f0bd8e14b0f7fc04a647f7609b17fdc0f
  with:
    script: tests/firmware-test.yaml
    version: v0.19.0
    output-dir: out/labwired
    args: --no-uart-stdout

- name: Link the automatic LabWired artifact
  if: always()
  run: echo "${{ steps.labwired.outputs.artifact-url }}" >> "$GITHUB_STEP_SUMMARY"
~~~

The action source is an immutable action-source pin to
`c6f8c68f0bd8e14b0f7fc04a647f7609b17fdc0f`. The `version` input defaults to
`v0.19.0` and independently selects the immutable Core CLI release archive
named `labwired-v0.19.0-<platform>.tar.gz`. The action downloads that public
archive from the `w1ne/labwired-core` GitHub release with `curl`; it does not
build Core on the consumer runner.

Its inputs are exactly `script` (required), `version` (default `v0.19.0`),
`output-dir`, and `args`. `args` is whitespace-separated extra CLI flags; shell
quoting inside this input is not interpreted.

Every invocation passes `--junit output-dir/junit.xml`, writes `summary.md` and
a self-contained `report.html` into `output-dir`, appends the Markdown report to
the GitHub job summary, and always uploads the entire output directory as an
artifact, including when the test fails.

Use the step outputs when a workflow needs to surface or link the generated
evidence: `status`, `summary-md`, `report-html`, `artifact-url`, and `exit-code`.
