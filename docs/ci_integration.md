# CI Integration

LabWired CI runs the same labwired test command locally, in GitHub Actions, and
in GitLab. Pin the runner release to v0.18.0 so a firmware change is tested
against a reproducible simulator version.

## GitHub Actions

Use the public LabWired Core action and select the Core CLI release with its
version input:

~~~yaml
name: Firmware simulation

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Build firmware
        run: cargo build --release --target thumbv7m-none-eabi -p firmware

      - id: labwired
        name: Run LabWired
        uses: w1ne/labwired-core/.github/actions/labwired-test@main
        with:
          version: v0.18.0
          script: tests/firmware-test.yaml
          output-dir: out/labwired
          args: --no-uart-stdout

      - name: Link the automatic LabWired artifact
        if: always()
        run: echo "${{ steps.labwired.outputs.artifact-url }}" >> "$GITHUB_STEP_SUMMARY"
~~~

The public action reference remains at main until a post-hardening Core action
tag is published. The version input is the immutable Core CLI release pin.
The action automatically appends `summary.md` to the job summary and uploads
the output directory plus the external JUnit file as an artifact, even when the
test fails. Its `status`, `summary-md`, `report-html`, `artifact-url`, and
`exit-code` outputs are available through the `labwired` step ID.

## Container runner

The release image has labwired as its entrypoint. Pass test directly after the
image name; do not repeat labwired in the container command:

~~~bash
docker run --rm \
  --volume "$PWD:/workspace" \
  --workdir /workspace \
  ghcr.io/w1ne/labwired:v0.18.0 \
  test --script tests/firmware-test.yaml \
       --output-dir out/labwired \
       --no-uart-stdout
~~~

The image runs as the runtime default user so it can write artifacts into the
mounted workspace.

## GitLab CI

GitLab must clear the image entrypoint so it can start its normal job shell.
The active template in [integration-templates/gitlab-ci.yml](integration-templates/gitlab-ci.yml)
uses the pinned image and then invokes labwired test.

~~~yaml
test:firmware:
  image:
    name: ghcr.io/w1ne/labwired:v0.18.0
    entrypoint: [""]
  script:
    - labwired test --script tests/firmware-test.yaml --output-dir out/labwired --no-uart-stdout
~~~

## Artifacts and reporting

Use --output-dir in every environment. A run writes result.json, snapshot.json,
uart.log, and junit.xml under that directory. The GitHub action automatically
adds a failure-safe job summary and artifact; other CI environments should
retain the directory so failed assertions keep their diagnostics.

## Advanced: build from source

Building labwired-cli from this repository is useful for testing an unreleased
commit or a local code change. It is intentionally an advanced alternative to
the pinned release archive or runner image:

~~~bash
cargo build --release -p labwired-cli
./target/release/labwired test --script tests/firmware-test.yaml --output-dir out/labwired
~~~

## Onboarding KPI tracking

For board onboarding competitiveness, core-onboarding-smoke.yml runs a
deterministic smoke path and emits onboarding-metrics.json,
onboarding-summary.md, onboarding-scoreboard.json, and
onboarding-scoreboard.md. Its soft 3600-second threshold tracks
time-to-first-smoke without blocking merges.
