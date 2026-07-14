#!/usr/bin/env bash
# Verify the static contract for the versioned LabWired CI runner release.
# This intentionally performs no network access and does not require Docker.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

workflow=.github/workflows/core-release.yml
backfill_workflow=.github/workflows/core-backfill-runner-image.yml
dockerfile=Dockerfile.ci
action=.github/actions/labwired-test/action.yml
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
require_file "$backfill_workflow"
require_file "$dockerfile"
require_file "$action"
require_file "$dockerignore"

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
runner_command_block=$(awk '
  /docker run --rm/ { inside = 1 }
  inside { print }
  inside && /test -s out\/release-runner-smoke\/result\.json/ { exit }
' <<<"$smoke_block")
require_block_literal "$runner_command_block" '"ghcr.io/w1ne/labwired:${{ github.ref_name }}"' 'release smoke runs the immutable image that it pulled'
require_block_absent_literal "$runner_command_block" 'ghcr.io/w1ne/labwired:latest' 'release smoke does not run the mutable latest tag'
require_block_literal "$smoke_block" 'examples/ci/dummy-max-steps.yaml' 'release smoke uses the deterministic CI script'
require_block_literal "$smoke_block" 'out/release-runner-smoke' 'release smoke writes runner artifacts to a dedicated directory'
require_block_literal "$smoke_block" '--no-uart-stdout' 'release smoke suppresses UART output'
require_block_literal "$smoke_block" 'test -s out/release-runner-smoke/result.json' 'release smoke asserts the runner result JSON exists and is nonempty'
require_block_literal "$smoke_block" 'uses: ./.github/actions/labwired-test' 'release smoke exercises the checked-out core action'
require_block_literal "$smoke_block" 'version: ${{ github.ref_name }}' 'release smoke pins the core action to the release tag'
require_block_literal "$smoke_block" 'github-token: ${{ github.token }}' 'release smoke explicitly passes its workflow token to the core action'
require_block_literal "$smoke_block" 'output-dir: out/release-action-smoke' 'release action smoke writes a dedicated output directory'
require_block_literal "$smoke_block" "upload-artifacts: 'false'" 'release action smoke avoids duplicate workflow artifacts'
require_block_literal "$smoke_block" 'test -s out/release-action-smoke/result.json' 'release smoke asserts the action result JSON exists and is nonempty'

require_literal "$dockerfile" 'FROM rust:1.95-slim AS builder' 'runner image builds with Rust 1.95'
require_literal "$dockerfile" 'RUN cargo build --release -p labwired-cli --locked' 'runner image builds only the CLI'
require_literal "$dockerfile" 'ARG VERSION' 'runner image accepts a release version build argument'
require_literal "$dockerfile" 'ARG REVISION' 'runner image accepts a source revision build argument'
require_literal "$dockerfile" 'org.opencontainers.image.source="https://github.com/w1ne/labwired-core"' 'runner image declares its OCI source label'
require_literal "$dockerfile" 'org.opencontainers.image.version="${VERSION}"' 'runner image declares its OCI version label'
require_literal "$dockerfile" 'org.opencontainers.image.revision="${REVISION}"' 'runner image declares its OCI revision label'
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

require_literal "$action" 'default: "v0.18.0"' 'core action defaults to the pinned supported release'
require_literal "$action" 'default: "w1ne/labwired-core"' 'core action downloads release archives from the core repository'
require_literal "$action" 'default: ${{ github.token }}' 'core action defaults its archive download token to the workflow token'
require_literal "$action" 'GH_TOKEN: ${{ inputs.github-token }}' 'core action passes an explicit or default token to gh release download'
require_literal "$action" 'LABWIRED_VERSION: ${{ inputs.version }}' 'core action passes the version through an environment binding'
require_literal "$action" 'LABWIRED_REPO: ${{ inputs.repo }}' 'core action passes the repository through an environment binding'
require_literal "$action" 'LABWIRED_SCRIPT: ${{ inputs.script }}' 'core action passes the script through an environment binding'
require_literal "$action" 'LABWIRED_ARGS: ${{ inputs.args }}' 'core action passes extra args through an environment binding'
require_literal "$action" 'command=(labwired test' 'core action constructs the test command as a Bash array'
require_literal "$action" 'read -r -a extra_args <<< "$LABWIRED_ARGS"' 'core action splits extra args without shell evaluation'
require_absent_literal "$action" "ver='\${{ inputs.version }}'" 'core action does not splice the version input into Bash source'
require_absent_literal "$action" "repo='\${{ inputs.repo }}'" 'core action does not splice the repository input into Bash source'
require_absent_literal "$action" "--script '\${{ inputs.script }}'" 'core action does not splice the script input into Bash source'
require_absent_literal "$action" '          ${{ inputs.args }}' 'core action does not splice extra args into Bash source'
repo_validation_line=$(grep -n -m 1 'repo must use the owner/name form' "$action" | cut -d: -f1 || true)
latest_lookup_line=$(grep -n -m 1 'gh release view --repo' "$action" | cut -d: -f1 || true)
if [[ -z "$repo_validation_line" || -z "$latest_lookup_line" || "$repo_validation_line" -ge "$latest_lookup_line" ]]; then
  fail 'core action validates the release repository before using GH_TOKEN with gh release view'
fi

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

for doc in docs/ci_integration.md docs/ci_test_runner.md docs/integration-templates/github-actions.yml docs/integration-templates/gitlab-ci.yml docs/integration-templates/README.md; do
  require_absent_literal "$doc" 'ghcr.io/w1ne/labwired:latest' "$doc does not recommend a mutable runner image tag"
done
require_literal docs/ci_integration.md 'w1ne/labwired/.github/actions/labwired-test@main' 'CI guide uses the published root GitHub action'
require_literal docs/ci_integration.md 'version: v0.18.0' 'CI guide pins the CLI release independently of the root action ref'
require_literal docs/integration-templates/github-actions.yml 'w1ne/labwired/.github/actions/labwired-test@main' 'GitHub template uses the published root action'
require_literal docs/integration-templates/github-actions.yml 'version: v0.18.0' 'GitHub template pins the CLI release independently of the root action ref'
require_literal docs/integration-templates/gitlab-ci.yml 'name: ghcr.io/w1ne/labwired:v0.18.0' 'GitLab template uses the pinned runner image'
require_literal docs/integration-templates/gitlab-ci.yml 'entrypoint: [""]' 'GitLab template clears the image entrypoint before invoking labwired'
require_literal .github/actions/labwired-test/README.md 'w1ne/labwired/.github/actions/labwired-test@main' 'core action README directs users to the published root action'
require_literal .github/actions/labwired-test/README.md 'version: v0.18.0' 'core action README pins the immutable CLI release separately from action source'

if (( failures > 0 )); then
  printf 'Release runner contract failed with %d issue(s).\n' "$failures" >&2
  exit 1
fi

printf 'Release runner contract passed.\n'
