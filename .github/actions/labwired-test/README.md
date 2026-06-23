# `labwired-test` action

Boot firmware on LabWired and assert against it, in one workflow step.

```yaml
- uses: w1ne/labwired/.github/actions/labwired-test@main
  with:
    script: examples/f103-fidelity-bench/clockbug-smoke.yaml
```

Installs the matching `labwired` release for the runner, runs `labwired test`,
writes a JUnit report, and uploads `result.json` + `uart.log` as an artifact.
The step fails when an assertion fails.

Inputs: `script` (required); `version` (default `latest`), `args`, `junit`,
`output-dir`, `upload-artifacts`, `repo`, `github-token`.
