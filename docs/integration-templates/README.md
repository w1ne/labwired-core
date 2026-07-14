# CI workflow templates

These templates run a pinned LabWired Core v0.19.1 release and preserve the
result.json, uart.log, snapshot.json, and junit.xml artifacts.

## GitHub Actions

[github-actions.yml](github-actions.yml) is the primary GitHub template. It
uses the public Core action at
w1ne/labwired-core/.github/actions/labwired-test@fda6a7bfb0328d9909ee07ba53ed05c84901f627
as an immutable action-source pin, while `version: v0.19.1` independently pins
the Core CLI. Its only inputs are `script` (required), `version`, `output-dir`,
and whitespace-separated `args`. The action downloads the public release archive
with `curl`, writes JUnit at `output-dir/junit.xml`, renders the GitHub report,
and always uploads the output directory.

Copy it into a firmware repository, then replace your-firmware and the test
script path:

~~~bash
cp docs/integration-templates/github-actions.yml .github/workflows/firmware-test.yml
~~~

## GitLab CI

[gitlab-ci.yml](gitlab-ci.yml) is active as written. Its test job uses:

~~~yaml
image:
  name: ghcr.io/w1ne/labwired:v0.19.1
  entrypoint: [""]
~~~

The empty entrypoint lets GitLab run its job shell, where the template calls
labwired test. Copy it to the repository root and replace your-firmware plus
the test script path.

## Direct Docker use

For local or non-GitHub CI runs, the release image keeps labwired as its
entrypoint:

~~~bash
docker run --rm -v "$PWD:/workspace" -w /workspace \
  ghcr.io/w1ne/labwired:v0.19.1 \
  test --script tests/firmware-test.yaml --output-dir out/labwired --no-uart-stdout
~~~

The Docker command and GitHub action invoke the same test YAML. It may describe
one machine directly or a world through `inputs.env`.

## Advanced source builds

Use a source build only to validate an unreleased LabWired revision. Normal CI
should use the pinned action or runner image so release behavior is
reproducible.

~~~bash
cargo build --release -p labwired-cli
./target/release/labwired test --script tests/firmware-test.yaml --output-dir out/labwired
~~~
