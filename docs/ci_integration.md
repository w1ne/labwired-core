# CI Integration

LabWired CI runs the same labwired test command locally, in GitHub Actions, and
in GitLab. Pin the runner release to v0.19.0 so a firmware change is tested
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

The public action reference is an immutable action-source pin to
`c6f8c68f0bd8e14b0f7fc04a647f7609b17fdc0f`. Its only inputs are `script`
(required), `version` (default `v0.19.0`), `output-dir`, and `args`; it downloads
the selected public CLI release archive with `curl`. The action writes JUnit to
`output-dir/junit.xml`, appends `summary.md` to the job summary, and always
uploads the entire output directory, even when the test fails. Its `status`,
`summary-md`, `report-html`, `artifact-url`, and `exit-code` outputs are
available through the `labwired` step ID.

## Container runner

The release image has labwired as its entrypoint. Pass test directly after the
image name; do not repeat labwired in the container command:

~~~bash
docker run --rm \
  --volume "$PWD:/workspace" \
  --workdir /workspace \
  ghcr.io/w1ne/labwired:v0.19.0 \
  test --script tests/firmware-test.yaml \
       --output-dir out/labwired \
       --no-uart-stdout
~~~

The image runs as the runtime default user so it can write artifacts into the
mounted workspace. The Docker command and GitHub action accept the same test
YAML: it can be a single-machine script or a world script that selects its
environment through `inputs.env`.

## GitLab CI

GitLab must clear the image entrypoint so it can start its normal job shell.
The active template in [integration-templates/gitlab-ci.yml](integration-templates/gitlab-ci.yml)
uses the pinned image and then invokes labwired test.

~~~yaml
test:firmware:
  image:
    name: ghcr.io/w1ne/labwired:v0.19.0
    entrypoint: [""]
  script:
    - labwired test --script tests/firmware-test.yaml --output-dir out/labwired --no-uart-stdout
~~~

## Artifacts and reporting

Use --output-dir in every environment. A run writes result.json, snapshot.json,
uart.log, and JUnit output under that directory. The GitHub action fixes the
JUnit path at `output-dir/junit.xml`, adds a failure-safe job summary and report,
and always uploads the directory; other CI environments should retain the
directory so failed assertions keep their diagnostics.

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
