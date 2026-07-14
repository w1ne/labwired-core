# CI Integration

LabWired CI runs the same labwired test command locally, in GitHub Actions, and
in GitLab. Pin the runner release to v0.18.0 so a firmware change is tested
against a reproducible simulator version.

## GitHub Actions

Use the public LabWired action at its published root location and select the
Core CLI release with the version input:

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

      - name: Run LabWired
        uses: w1ne/labwired/.github/actions/labwired-test@main
        with:
          version: v0.18.0
          script: tests/firmware-test.yaml
          output_dir: out/labwired
          # Optional: api-key: ${{ secrets.LABWIRED_API_KEY }}

      - name: Upload LabWired artifacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: labwired-results
          path: out/labwired
          if-no-files-found: warn
~~~

The public action reference remains at main because its repository has no
v0.18.0 tag. The version input is the immutable Core CLI release pin.

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
uart.log, and junit.xml under that directory. Upload the directory with an
always/failure-safe artifact step so failed assertions retain their diagnostics.

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
