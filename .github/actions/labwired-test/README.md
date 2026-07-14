# Core labwired-test action

This directory contains the public Core action used by release smoke tests and
consumer workflows. It downloads the pinned Core release archive rather than
compiling Rust on the consumer's runner.

~~~yaml
- id: labwired
  name: Run LabWired tests
  uses: w1ne/labwired-core/.github/actions/labwired-test@3a13349ad6c4f65b4fa19276f576bc3086b219e6
  with:
    version: v0.18.0
    script: tests/firmware-test.yaml
    output-dir: out/labwired
    args: --no-uart-stdout

- name: Link the automatic LabWired artifact
  if: always()
  run: echo "${{ steps.labwired.outputs.artifact-url }}" >> "$GITHUB_STEP_SUMMARY"
~~~

The action source is an immutable action-source pin to
`3a13349ad6c4f65b4fa19276f576bc3086b219e6`. The `version: v0.18.0` input
independently pins the immutable Core CLI release archive named
`labwired-v0.18.0-<platform>.tar.gz`.

The local Core action still uses its hyphenated names for its internal release
smoke workflow. Its inputs are `script` (required), `version`, `args`, `junit`,
`output-dir`, `upload-artifacts`, `repo`, and `github-token`.

Every invocation writes `summary.md` and a self-contained `report.html` into
`output-dir`, appends the Markdown report to the GitHub job summary, and uploads
the output directory plus the external JUnit path as an artifact by default.
Set `upload-artifacts: 'false'` only when a caller deliberately does not need a
workflow artifact.

Use the step outputs when a workflow needs to surface or link the generated
evidence: `status`, `summary-md`, `report-html`, `artifact-url`, and
`exit-code`. `artifact-url` is empty when artifact uploads are disabled.
