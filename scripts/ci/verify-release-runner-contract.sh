#!/usr/bin/env bash
# Verify the static contract for the versioned LabWired CI runner release.
# This intentionally performs no network access and does not require Docker.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

workflow=.github/workflows/core-release.yml
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
require_file "$dockerfile"
require_file "$action"
require_file "$dockerignore"

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
require_block_literal "$publish_block" 'registry: ghcr.io' 'publish logs into ghcr.io'
require_block_literal "$publish_block" 'ghcr.io/w1ne/labwired:${{ github.ref_name }}' 'publish includes the immutable release tag'
require_block_literal "$publish_block" 'ghcr.io/w1ne/labwired:latest' 'publish updates the latest tag'
require_block_literal "$publish_block" 'platforms: linux/amd64' 'publish initially targets linux/amd64'
require_block_literal "$publish_block" 'file: ./Dockerfile.ci' 'publish uses the CI runner Dockerfile'
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

for doc in docs/ci_integration.md docs/ci_test_runner.md docs/integration-templates/github-actions.yml docs/integration-templates/gitlab-ci.yml docs/integration-templates/README.md; do
  require_absent_literal "$doc" 'ghcr.io/w1ne/labwired:latest' "$doc does not recommend a mutable runner image tag"
done
require_literal docs/ci_integration.md 'w1ne/labwired/.github/actions/labwired-test@main' 'CI guide uses the published root GitHub action'
require_literal docs/ci_integration.md 'version: v0.18.0' 'CI guide pins the CLI release independently of the root action ref'
require_literal docs/integration-templates/github-actions.yml 'w1ne/labwired/.github/actions/labwired-test@main' 'GitHub template uses the published root action'
require_literal docs/integration-templates/github-actions.yml 'version: v0.18.0' 'GitHub template pins the CLI release independently of the root action ref'
require_literal docs/integration-templates/gitlab-ci.yml 'name: ghcr.io/w1ne/labwired:v0.18.0' 'GitLab template uses the pinned runner image'
require_literal docs/integration-templates/gitlab-ci.yml 'entrypoint: [""]' 'GitLab template clears the image entrypoint before invoking labwired'
require_literal .github/actions/labwired-test/README.md 'w1ne/labwired-core/.github/actions/labwired-test@v0.18.0' 'core action README names the core action and its pinned release'

if (( failures > 0 )); then
  printf 'Release runner contract failed with %d issue(s).\n' "$failures" >&2
  exit 1
fi

printf 'Release runner contract passed.\n'
