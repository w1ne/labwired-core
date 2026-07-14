#!/usr/bin/env bash
# Verify the static contract for the versioned LabWired CI runner release.
# This intentionally performs no network access and does not require Docker.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

workflow=.github/workflows/core-release.yml
core_ci_workflow=.github/workflows/core-ci.yml
backfill_workflow=.github/workflows/core-backfill-runner-image.yml
dockerfile=Dockerfile.ci
action=.github/actions/labwired-test/action.yml
renderer=.github/actions/labwired-test/render_report.py
renderer_test=.github/actions/labwired-test/test_render_report.py
environment_smoke_script=examples/ci/two-node-inputs-env.yaml
environment_smoke_manifest=examples/ci/two-node-env.yaml
dockerignore=.dockerignore
failures=0

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  failures=$((failures + 1))
}

require_file() {
  local file=$1
  if [[ ! -f "$file" ]]; then
    fail "missing required file: $file"
  fi
}

require_literal() {
  local file=$1
  local literal=$2
  local description=$3
  if ! grep -Fq -- "$literal" "$file"; then
    fail "$description (expected: $literal)"
  fi
}

require_absent_literal() {
  local file=$1
  local literal=$2
  local description=$3
  if grep -Fq -- "$literal" "$file"; then
    fail "$description (unexpected: $literal)"
  fi
}

require_block_literal() {
  local block=$1
  local literal=$2
  local description=$3
  if ! grep -Fq -- "$literal" <<<"$block"; then
    fail "$description (expected: $literal)"
  fi
}

require_block_absent_literal() {
  local block=$1
  local literal=$2
  local description=$3
  if grep -Fq -- "$literal" <<<"$block"; then
    fail "$description (unexpected: $literal)"
  fi
}

job_block() {
  local job=$1
  awk -v job="$job" '
    $0 == "  " job ":" { inside = 1; next }
    inside && $0 ~ /^  [[:alnum:]_-]+:$/ { exit }
    inside { print }
  ' "$workflow"
}

job_line() {
  grep -n -m 1 "^  $1:$" "$workflow" | cut -d: -f1
}

require_file "$workflow"
require_file "$core_ci_workflow"
require_file "$backfill_workflow"
require_file "$dockerfile"
require_file "$action"
require_file "$renderer"
require_file "$renderer_test"
require_file "$environment_smoke_script"
require_file "$environment_smoke_manifest"
require_file "$dockerignore"

if ! python3 "$renderer_test"; then
  fail 'report renderer unit tests pass'
fi

release_runner_contract_block=$(awk '
  $0 == "  release-runner-contract:" { inside = 1; next }
  inside && $0 ~ /^  [[:alnum:]_-]+:$/ { exit }
  inside { print }
' "$core_ci_workflow")
require_block_literal "$release_runner_contract_block" 'actions/checkout@v4' 'release runner contract job checks out the source'
require_block_literal "$release_runner_contract_block" 'fetch-depth: 0' 'release runner contract job fetches immutable action-source pins'

core_integrity_block=$(awk '
  $0 == "  integrity:" { inside = 1; next }
  inside && $0 ~ /^  [[:alnum:]_-]+:$/ { exit }
  inside { print }
' "$core_ci_workflow")
if [[ -z "$core_integrity_block" ]]; then
  fail 'core-integrity job is present in Core CI'
fi
require_block_literal "$core_integrity_block" 'docker build --pull' 'required Core integrity builds the runner image from fresh base layers'
require_block_literal "$core_integrity_block" '--file Dockerfile.ci' 'required Core integrity uses the CI runner Dockerfile'
require_block_literal "$core_integrity_block" 'labwired-ci-smoke:local' 'required Core integrity gives the local image a stable tag'
require_block_literal "$core_integrity_block" 'VERSION=ci-smoke' 'required Core integrity provides OCI version metadata'
require_block_literal "$core_integrity_block" 'REVISION="$GITHUB_SHA"' 'required Core integrity provides OCI revision metadata'
require_block_literal "$core_integrity_block" 'docker run --rm labwired-ci-smoke:local --version' 'required Core integrity executes the final image entrypoint'
require_block_absent_literal "$core_integrity_block" 'docker/login-action@v3' 'required Core integrity does not need registry credentials'
require_block_absent_literal "$core_integrity_block" 'docker/build-push-action@v6' 'required Core integrity does not publish an untagged PR image'

require_literal "$workflow" 'tags:' 'release workflow declares a tag trigger'
require_literal "$workflow" "'v[0-9]+.[0-9]+.[0-9]+'" 'release workflow triggers vMAJOR.MINOR.PATCH tags'
for target in \
  x86_64-unknown-linux-gnu \
  aarch64-unknown-linux-gnu \
  x86_64-apple-darwin \
  aarch64-apple-darwin; do
  require_literal "$workflow" "target: $target" "archive matrix includes $target"
done
for platform in linux-x86_64 linux-aarch64 darwin-x86_64 darwin-aarch64; do
  require_literal "$workflow" "platform: $platform" "archive matrix includes $platform"
done
require_literal "$workflow" 'ARCHIVE="labwired-${VERSION}-${PLATFORM}.tar.gz"' 'archive names include the release version and platform'
require_literal "$workflow" 'cp "target/${{ matrix.target }}/release/labwired" dist/labwired' 'archive package copies the labwired binary'
require_literal "$workflow" 'tar -czf "${ARCHIVE}" -C dist labwired' 'archive package contains the labwired binary'
require_literal "$workflow" 'name: labwired-${{ matrix.platform }}' 'archive artifact names follow the platform matrix'

release_block=$(job_block release)
require_block_literal "$release_block" 'needs: build' 'release waits for all archive builds'
require_block_literal "$release_block" 'softprops/action-gh-release@v2' 'release creates the GitHub release with action-gh-release'
require_block_literal "$release_block" 'files: dist/*.tar.gz' 'release uploads every generated archive asset'

require_literal "$workflow" 'packages: write' 'release workflow grants GHCR package publishing permission'
require_literal "$workflow" 'docker/setup-buildx-action@v3' 'publish job configures Docker Buildx'
require_literal "$workflow" 'docker/login-action@v3' 'publish job logs into GHCR'
require_literal "$workflow" 'docker/build-push-action@v6' 'publish job pushes the runner image'

release_line=$(job_line release || true)
publish_line=$(job_line publish || true)
if [[ -z "$release_line" || -z "$publish_line" || "$publish_line" -le "$release_line" ]]; then
  fail 'publish job appears after the archive release job'
fi

publish_block=$(job_block publish)
require_block_literal "$publish_block" 'needs: release' 'publish waits for archive release completion'
require_block_literal "$publish_block" 'docker/setup-buildx-action@v3' 'publish configures Buildx in the publish job'
require_block_literal "$publish_block" 'docker/login-action@v3' 'publish logs into GHCR in the publish job'
require_block_literal "$publish_block" 'docker/build-push-action@v6' 'publish uses build-push-action in the publish job'
require_block_literal "$publish_block" 'registry: ghcr.io' 'publish logs into ghcr.io'
require_block_literal "$publish_block" 'context: .' 'publish builds from the repository context'
require_block_literal "$publish_block" 'ghcr.io/w1ne/labwired:${{ github.ref_name }}' 'publish includes the immutable release tag'
require_block_literal "$publish_block" 'ghcr.io/w1ne/labwired:latest' 'publish updates the latest tag'
require_block_literal "$publish_block" 'platforms: linux/amd64' 'publish initially targets linux/amd64'
require_block_literal "$publish_block" 'file: ./Dockerfile.ci' 'publish uses the CI runner Dockerfile'
require_block_literal "$publish_block" 'push: true' 'publish pushes the runner image to GHCR'
require_block_literal "$publish_block" 'VERSION=${{ github.ref_name }}' 'publish passes the OCI version build argument'
require_block_literal "$publish_block" 'REVISION=${{ github.sha }}' 'publish passes the OCI revision build argument'

smoke_block=$(job_block release-smoke)
if [[ -z "$smoke_block" ]]; then
  fail 'release-smoke job is present'
fi
require_block_literal "$smoke_block" 'needs: [release, publish]' 'release smoke waits for both archive release and image publish'
require_block_literal "$smoke_block" 'actions/checkout@v4' 'release smoke checks out the test script before mounting it'
require_block_literal "$smoke_block" 'docker logout ghcr.io || true' 'release smoke intentionally clears GHCR credentials before the pull'
require_block_absent_literal "$smoke_block" 'docker/login-action@v3' 'release smoke remains anonymous for the GHCR pull'
require_block_literal "$smoke_block" 'docker pull "ghcr.io/w1ne/labwired:${{ github.ref_name }}"' 'release smoke anonymously pulls the immutable image'
require_block_literal "$smoke_block" 'Make the GHCR package public' 'release smoke explains how to fix a private first GHCR package'
require_block_literal "$smoke_block" 'docker run --rm' 'release smoke runs the image directly'
require_block_absent_literal "$smoke_block" 'cargo build' 'release smoke does not build Core from source'
require_block_absent_literal "$smoke_block" 'cross build' 'release smoke does not cross-build Core from source'
runner_command_block=$(awk '
  /docker run --rm/ { inside = 1 }
  inside { print }
  inside && /test -s out\/release-runner-smoke\/result\.json/ { exit }
' <<<"$smoke_block")
require_block_literal "$runner_command_block" '"ghcr.io/w1ne/labwired:${{ github.ref_name }}"' 'release smoke runs the immutable image that it pulled'
require_block_absent_literal "$runner_command_block" 'ghcr.io/w1ne/labwired:latest' 'release smoke does not run the mutable latest tag'
require_block_literal "$smoke_block" 'examples/ci/two-node-inputs-env.yaml' 'release smoke uses the explicit two-node inputs.env script'
require_block_literal "$smoke_block" 'out/release-runner-smoke' 'release smoke writes runner artifacts to a dedicated directory'
require_block_literal "$smoke_block" '--no-uart-stdout' 'release smoke suppresses UART output'
require_block_literal "$smoke_block" 'test -s out/release-runner-smoke/result.json' 'release smoke asserts the runner result JSON exists and is nonempty'
require_block_literal "$smoke_block" 'test -s out/release-runner-smoke/junit.xml' 'release smoke asserts the OCI runner writes JUnit in its output directory'
require_block_literal "$smoke_block" 'uses: ./.github/actions/labwired-test' 'release smoke exercises the checked-out core action'
require_block_literal "$smoke_block" 'version: ${{ github.ref_name }}' 'release smoke pins the core action to the release tag'
require_block_absent_literal "$smoke_block" 'github-token:' 'release smoke does not need an archive-download token'
require_block_literal "$smoke_block" 'output-dir: out/release-action-smoke' 'release action smoke writes a dedicated output directory'
require_block_absent_literal "$smoke_block" 'upload-artifacts:' 'release action smoke cannot disable artifact uploads'
require_block_literal "$smoke_block" 'test -s out/release-action-smoke/result.json' 'release smoke asserts the action result JSON exists and is nonempty'
require_block_literal "$smoke_block" 'test -s out/release-action-smoke/junit.xml' 'release smoke asserts the action writes JUnit in its output directory'
release_action_invocations=$(grep -F -c 'uses: ./.github/actions/labwired-test' <<<"$smoke_block" || true)
if [[ "$release_action_invocations" -ne 2 ]]; then
  fail 'release smoke invokes the core action twice in one job to prove artifact names do not collide'
fi
require_block_literal "$smoke_block" 'output-dir: out/release-action-smoke-second' 'release smoke gives the second action invocation its own output directory'
require_block_literal "$smoke_block" 'test -s out/release-action-smoke-second/result.json' 'release smoke asserts the second action result JSON exists and is nonempty'
require_block_literal "$smoke_block" 'test -s out/release-action-smoke-second/junit.xml' 'release smoke asserts the second action writes JUnit in its output directory'

require_literal "$dockerfile" 'FROM rust:1.95-slim-bookworm AS builder' 'runner image builds with Rust 1.95 on bookworm'
require_literal "$dockerfile" 'FROM debian:bookworm-slim' 'runner image runtime matches the builder libc baseline'
require_literal "$dockerfile" 'RUN cargo build --release -p labwired-cli --locked' 'runner image builds only the CLI'
require_literal "$dockerfile" 'ARG VERSION' 'runner image accepts a release version build argument'
require_literal "$dockerfile" 'ARG REVISION' 'runner image accepts a source revision build argument'
require_literal "$dockerfile" 'org.opencontainers.image.source="https://github.com/w1ne/labwired-core"' 'runner image declares its OCI source label'
require_literal "$dockerfile" 'org.opencontainers.image.version="${VERSION}"' 'runner image declares its OCI version label'
require_literal "$dockerfile" 'org.opencontainers.image.revision="${REVISION}"' 'runner image declares its OCI revision label'
require_literal "$dockerfile" 'RUN labwired --version' 'runner image verifies the final runtime can execute the CLI'
require_literal "$dockerfile" 'ENTRYPOINT ["labwired"]' 'runner image preserves the labwired CLI entrypoint'
require_absent_literal "$dockerfile" 'USER ' 'runner image leaves the default user unchanged for bind-mounted output directories'
require_absent_literal "$dockerfile" 'labwired-dap' 'runner image does not ship the DAP binary'

if [[ -f "$dockerignore" ]]; then
  for ignored_path in .git target out; do
    if ! grep -Eq "^[[:space:]]*${ignored_path//./\\.}[[:space:]]*$" "$dockerignore"; then
      fail "Docker build context excludes $ignored_path"
    fi
  done
fi

require_literal "$action" 'default: "v0.19.0"' 'core action defaults to the supported public release'
action_inputs=$(awk '
  /^inputs:$/ { inside = 1; next }
  inside && /^[^[:space:]]/ { exit }
  inside && /^  [[:alnum:]][[:alnum:]-]*:$/ {
    key = $1
    sub(/:$/, "", key)
    print key
  }
' "$action" | sort)
if [[ "$action_inputs" != $'args\noutput-dir\nscript\nversion' ]]; then
  fail 'core action exposes exactly script, version, output-dir, and args inputs'
fi
if ! awk '
  /^  script:$/ { inside = 1; next }
  inside && /^  [[:alnum:]][[:alnum:]-]*:$/ { exit }
  inside && /^    required: true$/ { found = 1 }
  END { exit !found }
' "$action"; then
  fail 'core action requires the script input'
fi
require_literal "$action" 'https://github.com/w1ne/labwired-core/releases/download/${version}/${asset}' 'core action downloads the fixed public Core release archive'
require_literal "$action" 'curl --fail --location --retry 3 --retry-delay 2 --output "$archive" "$url"' 'core action downloads archives with curl'
require_literal "$action" 'version must be a vMAJOR.MINOR.PATCH release tag.' 'core action requires an immutable release version'
for removed_input in repo: junit: upload-artifacts: github-token:; do
  require_absent_literal "$action" "$removed_input" "core action exposes only the public one-step inputs, not $removed_input"
done
require_absent_literal "$action" 'GH_TOKEN' 'core action does not expose a release-download token to a shell'
require_absent_literal "$action" 'gh release' 'core action does not depend on the gh CLI'
require_absent_literal "$action" 'latest' 'core action does not resolve a mutable latest release'
require_literal "$action" 'LABWIRED_VERSION: ${{ inputs.version }}' 'core action passes the version through an environment binding'
require_literal "$action" 'LABWIRED_SCRIPT: ${{ inputs.script }}' 'core action passes the script through an environment binding'
require_literal "$action" 'LABWIRED_ARGS: ${{ inputs.args }}' 'core action passes extra args through an environment binding'
require_literal "$action" 'command=(labwired test' 'core action constructs the test command as a Bash array'
require_literal "$action" '--junit "$LABWIRED_OUTPUT_DIR/junit.xml"' 'core action always writes JUnit inside its output directory'
require_literal "$action" 'mkdir -p "$LABWIRED_OUTPUT_DIR"' 'core action prepares the output directory before running'
require_literal "$action" 'read -r -a extra_args <<< "$LABWIRED_ARGS"' 'core action splits extra args without shell evaluation'
require_absent_literal "$action" "ver='\${{ inputs.version }}'" 'core action does not splice the version input into Bash source'
require_absent_literal "$action" "--script '\${{ inputs.script }}'" 'core action does not splice the script input into Bash source'
require_absent_literal "$action" '          ${{ inputs.args }}' 'core action does not splice extra args into Bash source'

require_literal "$action" 'outputs:' 'core action exposes report outputs'
require_literal "$action" 'summary-md:' 'core action exposes a Markdown report output'
require_literal "$action" 'report-html:' 'core action exposes an HTML report output'
require_literal "$action" 'artifact-url:' 'core action exposes the uploaded artifact URL'
require_literal "$action" 'exit-code:' 'core action exposes the captured CLI exit code'
require_literal "$action" 'value: ${{ steps.report.outputs.status }}' 'core action exposes the renderer status'
require_literal "$action" 'value: ${{ steps.report.outputs.summary_md }}' 'core action exposes the Markdown report path'
require_literal "$action" 'value: ${{ steps.report.outputs.report_html }}' 'core action exposes the HTML report path'
require_literal "$action" 'value: ${{ steps.upload.outputs.artifact-url }}' 'core action exposes the uploaded artifact URL'
require_literal "$action" 'value: ${{ steps.run.outputs.exit_code }}' 'core action exposes the captured CLI exit code'
require_literal "$action" 'id: run' 'core action identifies the CLI run step'
require_literal "$action" 'id: install' 'core action identifies the release installation step'
require_literal "$action" 'id: report' 'core action identifies the report step'
require_literal "$action" 'id: upload' 'core action identifies the artifact upload step'
require_literal "$action" 'render_report.py' 'core action invokes the bundled report renderer'
require_literal "$action" 'GITHUB_STEP_SUMMARY' 'core action appends the report to the job summary'
require_literal "$action" 'cat "$LABWIRED_SUMMARY_MD" >> "$GITHUB_STEP_SUMMARY"' 'core action appends only the renderer-produced summary to the job summary'
require_literal "$action" 'if: ${{ always() }}' 'core action always renders and uploads artifacts after a failed test'
require_literal "$action" 'path: ${{ inputs.output-dir }}' 'core action uploads exactly the configured output directory'
require_literal "$action" 'if-no-files-found: error' 'core action treats a missing output directory as a workflow error'
require_literal "$action" 'name: labwired-${{ github.job }}-${{ github.run_id }}-${{ github.action }}' 'core action gives each invocation a unique artifact name'
require_literal "$action" 'LABWIRED_RUN_URL: ${{ github.server_url }}/${{ github.repository }}/actions/runs/${{ github.run_id }}' 'core action records the workflow run URL'
require_literal "$action" 'LABWIRED_SOURCE_REVISION: ${{ github.sha }}' 'core action records the source revision'
require_literal "$action" 'LABWIRED_RELEASE_VERSION: ${{ steps.install.outputs.release_version }}' 'core action records the resolved release'
require_literal "$action" 'echo "release_version=$version" >> "$GITHUB_OUTPUT"' 'core action records the resolved release version after installation'
require_literal "$action" 'LABWIRED_EXIT_CODE: ${{ steps.run.outputs.exit_code }}' 'core action safely passes the captured exit code to its final step'
require_literal "$action" 'exit "$LABWIRED_EXIT_CODE"' 'core action returns the captured LabWired failure code after reporting'

run_command_block=$'        set +e\n        "${command[@]}"\n        exit_code=$?\n        set -e\n        echo "exit_code=$exit_code" >> "$GITHUB_OUTPUT"\n        exit 0'
require_literal "$action" "$run_command_block" 'core action captures the CLI result before rendering and uploading reports'
run_command_count=$(grep -F -c '"${command[@]}"' "$action" || true)
if [[ "$run_command_count" -ne 1 ]]; then
  fail 'core action does not execute the test command directly outside the captured run lifecycle'
fi

require_literal "$renderer" 'RESULT_JSON_LIMIT_BYTES = 1024 * 1024' 'renderer caps result JSON reads at 1 MiB'
require_literal "$renderer" 'result_file.read(RESULT_JSON_LIMIT_BYTES + 1)' 'renderer reads only one bounded result JSON payload'
require_absent_literal "$renderer" 'path.read_text(encoding="utf-8")' 'renderer does not load result JSON without a byte cap'
require_literal "$renderer" 'ASSERTION_RENDER_LIMIT = 200' 'renderer caps rendered assertion rows'
require_literal "$renderer" 'raw_assertions[:ASSERTION_RENDER_LIMIT]' 'renderer selects only the bounded assertion prefix'
require_literal "$renderer" 'for assertion in raw_assertions:' 'renderer aggregates assertion outcomes beyond rendered rows'
require_literal "$renderer" 'DISPLAY_VALUE_LIMIT = 4096' 'renderer caps report field display values'
require_literal "$renderer" 'SUMMARY_LIMIT_BYTES = 64 * 1024' 'renderer caps the job-summary markdown payload'
require_literal "$renderer" 'Report summary truncated after 65536 bytes.' 'renderer marks a capped job summary'
require_literal "$renderer" 'cap_markdown_summary(summary)' 'renderer caps Markdown before the action appends it'
require_literal "$renderer" 'SCRIPT_HASH_CHUNK_BYTES = 64 * 1024' 'renderer hashes scripts in bounded chunks'
require_literal "$renderer" 'while chunk := script_file.read(SCRIPT_HASH_CHUNK_BYTES):' 'renderer streams script hashing'
require_absent_literal "$renderer" 'read_bytes()' 'renderer does not read a complete script into memory'
require_literal "$renderer" 'Message: {markdown_code(message)}' 'renderer preserves config-error messages in Markdown'
require_literal "$renderer" '<dt>Message</dt>' 'renderer preserves config-error messages in HTML'
require_literal "$renderer_test" 'test_oversized_result_is_not_rendered_and_is_diagnosed' 'renderer tests oversized result handling'
require_literal "$renderer_test" 'test_excessive_assertions_are_capped_and_marked' 'renderer tests assertion rendering caps'
require_literal "$renderer_test" 'test_omitted_assertion_failure_is_included_in_totals' 'renderer tests full assertion totals beyond rendered rows'
require_literal "$renderer_test" 'test_config_error_message_is_safely_rendered' 'renderer tests config-error diagnostics escaping'
require_literal "$renderer_test" 'test_large_script_hashes_without_reading_the_whole_file' 'renderer tests streamed script hashing'
require_literal "$renderer_test" 'test_markdown_summary_cap_is_bounded_and_marked' 'renderer tests job-summary caps'
require_literal "$renderer_test" 'test_renders_environment_result_without_single_machine_provenance' 'renderer tests the environment result union arm'
require_literal "$renderer_test" '"result_schema_version": "1.0-environment"' 'renderer environment fixture uses the environment result schema'
require_literal "$renderer_test" '"run_type": "environment"' 'renderer environment fixture distinguishes the environment run type'

require_literal "$environment_smoke_script" 'inputs:' 'release smoke fixture declares inputs'
require_literal "$environment_smoke_script" 'env: "two-node-env.yaml"' 'release smoke fixture selects the two-node environment through inputs.env'
require_literal "$environment_smoke_manifest" 'schema_version: "1.0"' 'release smoke environment uses the supported manifest schema'
two_node_count=$(grep -Ec '^[[:space:]]*-[[:space:]]id:' "$environment_smoke_manifest" || true)
if [[ "$two_node_count" -ne 2 ]]; then
  fail 'release smoke environment manifest declares exactly two nodes'
fi
require_literal "$environment_smoke_manifest" 'id: alpha' 'release smoke environment includes alpha'
require_literal "$environment_smoke_manifest" 'id: beta' 'release smoke environment includes beta'

require_literal "$backfill_workflow" 'workflow_dispatch:' 'runner backfill workflow requires explicit manual dispatch'
require_literal "$backfill_workflow" 'required: true' 'runner backfill requires an explicit release version'
require_literal "$backfill_workflow" 'type: string' 'runner backfill version is a string input'
require_literal "$backfill_workflow" '[[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]' 'runner backfill validates a semver release tag'
require_literal "$backfill_workflow" 'ref: ${{ inputs.version }}' 'runner backfill checks out the exact released source tag'
require_literal "$backfill_workflow" 'path: source' 'runner backfill keeps released source in a dedicated directory'
require_literal "$backfill_workflow" 'path: release-tools' 'runner backfill keeps current release tooling in a dedicated directory'
require_literal "$backfill_workflow" 'context: ./source' 'runner backfill builds the exact released source context'
require_literal "$backfill_workflow" 'file: ./release-tools/Dockerfile.ci' 'runner backfill uses current CI runner packaging instructions'
require_literal "$backfill_workflow" 'ghcr.io/w1ne/labwired:${{ inputs.version }}' 'runner backfill publishes the requested immutable image tag'
require_absent_literal "$backfill_workflow" 'ghcr.io/w1ne/labwired:latest' 'runner backfill never overwrites the moving latest tag'
require_literal "$backfill_workflow" 'docker logout ghcr.io || true' 'runner backfill verifies a public anonymous pull'
require_literal "$backfill_workflow" 'docker pull "ghcr.io/w1ne/labwired:${{ inputs.version }}"' 'runner backfill pulls the immutable tag after publication'
require_literal "$backfill_workflow" 'examples/ci/dummy-max-steps.yaml' 'runner backfill uses the deterministic CI smoke script'
require_literal RELEASE_PROCESS.md 'core-backfill-runner-image.yml' 'release process documents the one-time runner image backfill workflow'
require_literal RELEASE_PROCESS.md 'v0.18.0' 'release process documents the initial v0.18.0 runner image backfill'

safe_action_sha=82c6c78983669f8688f3823db9a81d1c2bdef202
safe_action_version=v0.19.0
safe_action_ref="w1ne/labwired-core/.github/actions/labwired-test@${safe_action_sha}"
for doc in docs/ci_integration.md docs/ci_test_runner.md docs/integration-templates/github-actions.yml docs/integration-templates/gitlab-ci.yml docs/integration-templates/README.md docs/reference_client_flows.md .github/actions/labwired-test/README.md; do
  require_absent_literal "$doc" 'ghcr.io/w1ne/labwired:latest' "$doc does not recommend a mutable runner image tag"
  require_absent_literal "$doc" 'w1ne/labwired/.github/actions/labwired-test@main' "$doc does not point public users at the private root action"
  require_absent_literal "$doc" '3a13349ad6c4f65b4fa19276f576bc3086b219e6' "$doc does not retain the superseded public action pin"
  require_absent_literal "$doc" 'version: v0.18.0' "$doc does not retain the superseded Core release version"
done
for doc in docs/ci_integration.md docs/ci_test_runner.md docs/integration-templates/github-actions.yml docs/integration-templates/README.md docs/reference_client_flows.md; do
  require_absent_literal "$doc" 'w1ne/labwired-core/.github/actions/labwired-test@main' "$doc does not use a mutable public Core action ref"
  require_absent_literal "$doc" 'labwired/setup-action@v1' "$doc does not use the obsolete setup action"
  require_literal "$doc" "$safe_action_ref" "$doc uses the immutable public Core action ref"
  require_literal "$doc" 'immutable action-source pin' "$doc explains the immutable action source pin"
  require_literal "$doc" "version: $safe_action_version" "$doc pins the Core CLI release independently of the action source"
done
require_literal docs/ci_integration.md "$safe_action_ref" 'CI guide uses the immutable public Core GitHub action'
require_literal docs/ci_integration.md "version: $safe_action_version" 'CI guide pins the CLI release independently of the public action ref'
require_literal docs/ci_integration.md 'output-dir: out/labwired' 'CI guide uses the Core action artifact input spelling'
require_literal docs/ci_integration.md 'steps.labwired.outputs.artifact-url' 'CI guide shows how to consume the automatic artifact URL'
require_literal docs/ci_integration.md 'if: always()' 'CI guide links the automatic artifact after failed tests'
require_absent_literal docs/ci_integration.md 'uses: actions/upload-artifact@v4' 'CI guide does not duplicate the action artifact upload'
require_literal docs/integration-templates/github-actions.yml "$safe_action_ref" 'GitHub template uses the immutable public Core action'
require_literal docs/integration-templates/github-actions.yml "version: $safe_action_version" 'GitHub template pins the CLI release independently of the public action ref'
require_literal docs/integration-templates/github-actions.yml 'output-dir: out/labwired' 'GitHub template uses the Core action artifact input spelling'
require_literal docs/integration-templates/github-actions.yml 'steps.labwired.outputs.artifact-url' 'GitHub template shows how to consume the automatic artifact URL'
require_literal docs/integration-templates/github-actions.yml 'if: always()' 'GitHub template links the automatic artifact after failed tests'
require_absent_literal docs/integration-templates/github-actions.yml 'uses: actions/upload-artifact@v4' 'GitHub template does not duplicate the action artifact upload'
require_literal docs/ci_test_runner.md 'steps.labwired.outputs.artifact-url' 'runner guide shows how to consume the automatic artifact URL'
require_literal docs/ci_test_runner.md 'if: always()' 'runner guide links the automatic artifact after failed tests'
require_absent_literal docs/ci_test_runner.md 'uses: actions/upload-artifact@v4' 'runner guide does not duplicate the action artifact upload'
require_literal docs/integration-templates/gitlab-ci.yml "name: ghcr.io/w1ne/labwired:$safe_action_version" 'GitLab template uses the pinned runner image'
require_literal docs/integration-templates/gitlab-ci.yml 'entrypoint: [""]' 'GitLab template clears the image entrypoint before invoking labwired'

if ! git cat-file -e "${safe_action_sha}^{commit}" 2>/dev/null; then
  fail "immutable action-source commit $safe_action_sha is available locally"
else
  pinned_action=''
  pinned_action_readme=''
  if ! pinned_action=$(git show "${safe_action_sha}:${action}" 2>/dev/null); then
    fail "immutable action-source commit $safe_action_sha contains $action"
  fi
  if ! pinned_action_readme=$(git show "${safe_action_sha}:.github/actions/labwired-test/README.md" 2>/dev/null); then
    fail "immutable action-source commit $safe_action_sha contains its action README"
  fi
  if [[ -n "$pinned_action" ]]; then
    require_block_literal "$pinned_action" 'default: "v0.19.0"' 'pinned action defaults to the supported public release'
    require_block_literal "$pinned_action" 'https://github.com/w1ne/labwired-core/releases/download/${version}/${asset}' 'pinned action downloads the public release archive'
    pinned_action_inputs=$(awk '
      /^inputs:$/ { inside = 1; next }
      inside && /^[^[:space:]]/ { exit }
      inside && /^  [[:alnum:]][[:alnum:]-]*:$/ {
        key = $1
        sub(/:$/, "", key)
        print key
      }
    ' <<<"$pinned_action" | sort)
    if [[ "$pinned_action_inputs" != $'args\noutput-dir\nscript\nversion' ]]; then
      fail 'pinned action exposes exactly script, version, output-dir, and args inputs'
    fi
    for removed_input in repo: junit: upload-artifacts: github-token:; do
      require_block_absent_literal "$pinned_action" "$removed_input" "pinned action does not expose retired $removed_input input"
    done
    require_block_literal "$pinned_action" 'if: ${{ always() }}' 'pinned action always renders and uploads reports'
    require_block_literal "$pinned_action" 'name: labwired-${{ github.job }}-${{ github.run_id }}-${{ github.action }}' 'pinned action gives each invocation a unique artifact name'
  fi
  if [[ -n "$pinned_action_readme" ]]; then
    require_block_literal "$pinned_action_readme" '[CI integration guide](../../../docs/ci_integration.md)' 'pinned action README links the canonical consumer guide'
    require_block_literal "$pinned_action_readme" 'intentionally documents the action beside its implementation' 'pinned action README explains its source-local contract'
    require_block_absent_literal "$pinned_action_readme" 'uses: w1ne/labwired-core/.github/actions/labwired-test@' 'pinned action README does not recursively choose its own SHA'
    for retired_input in repo: github-token: junit: upload-artifacts:; do
      require_block_absent_literal "$pinned_action_readme" "$retired_input" "pinned action README does not document retired $retired_input input"
    done
  fi
fi

require_literal docs/ci_test_runner.md 'oneOf' 'runner guide documents the single-machine/environment result union'
require_literal docs/ci_test_runner.md '"1.0-environment"' 'runner guide documents the environment result schema version'
require_literal docs/ci_test_runner.md '"run_type"' 'runner guide documents the environment run discriminator'
require_literal docs/ci_test_runner.md 'world_firmware_hash' 'runner guide documents world provenance'
require_literal docs/ci_test_runner.md 'inputs.env' 'runner guide documents environment test inputs'
require_literal docs/ci_test_runner.md 'config.peripheral' 'runner guide documents the CAN-bus peripheral requirement'
require_literal docs/ci_test_runner.md 'Cortex-M-only' 'runner guide documents the current world architecture boundary'
require_literal docs/ci_test_runner.md 'core: cortex-m*' 'runner guide documents the explicit Cortex-M core requirement'
require_literal docs/ci_test_runner.md 'Cortex-M Thumb reset vector' 'runner guide documents the firmware vector requirement'
require_literal docs/ci_test_runner.md 'config_overrides' 'runner guide documents that per-node overrides are rejected'
require_literal docs/ci_test_runner.md 'including `{}` and `null`' 'runner guide documents that explicit empty and null overrides are rejected'
require_literal docs/ci_test_runner.md 'uart_cross_link' 'runner guide documents UART cross-link membership validation'
require_literal docs/ci_test_runner.md 'egress' 'runner guide documents egress membership validation'
require_literal docs/ci_test_runner.md 'Each `config` mapping is closed and type-checked' 'runner guide documents strict interconnect config mappings'
require_literal docs/simulation_protocol.md 'schema_version: "1.0"' 'simulation protocol documents the released environment schema'
require_literal docs/simulation_protocol.md 'world_firmware_hash' 'simulation protocol documents environment provenance'
require_literal docs/simulation_protocol.md 'config.peripheral' 'simulation protocol documents the CAN-bus peripheral requirement'
require_literal docs/simulation_protocol.md 'Cortex-M-only' 'simulation protocol documents the current world architecture boundary'
require_literal docs/simulation_protocol.md 'core: cortex-m*' 'simulation protocol documents the explicit Cortex-M core requirement'
require_literal docs/simulation_protocol.md 'Cortex-M Thumb reset vector' 'simulation protocol documents the firmware vector requirement'
require_literal docs/simulation_protocol.md 'config_overrides' 'simulation protocol documents that per-node overrides are rejected'
require_literal docs/simulation_protocol.md 'including `{}` and `null`' 'simulation protocol documents that explicit empty and null overrides are rejected'
require_literal docs/simulation_protocol.md 'uart_cross_link' 'simulation protocol documents UART cross-link membership validation'
require_literal docs/simulation_protocol.md 'egress' 'simulation protocol documents egress membership validation'
require_literal docs/simulation_protocol.md 'strict and closed' 'simulation protocol documents strict interconnect config mappings'
require_absent_literal docs/simulation_protocol.md 'Future v1.1' 'simulation protocol does not label released environments as future work'
require_literal docs/configuration_reference.md 'core: cortex-m*' 'configuration reference documents the explicit Cortex-M core requirement'
require_literal docs/configuration_reference.md 'including `{}` and `null`' 'configuration reference documents explicit empty and null override rejection'
require_literal examples/egress-demo/README.md 'config` is a closed mapping' 'egress example documents its closed config mapping'
require_literal examples/egress-demo/README.md 'positive integer' 'egress example documents buffer_max type validation'
require_literal README.md 'LABWIRED_VERSION=v0.19.0' 'public README pins the current release version'
require_absent_literal README.md 'LABWIRED_VERSION=v0.18.0' 'public README does not retain the superseded release version'

if (( failures > 0 )); then
  printf 'Release runner contract failed with %d issue(s).\n' "$failures" >&2
  exit 1
fi

printf 'Release runner contract passed.\n'
