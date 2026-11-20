#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${REPO_ROOT}"

"${REPO_ROOT}/scripts/onboarding_smoke.sh" \
  --target-id lp-mspm0g3507-launchpad \
  --crate firmware-lp-mspm0g3507-smoke \
  --target thumbv6m-none-eabi \
  --script examples/lp-mspm0g3507-launchpad/io-smoke.yaml \
  --system configs/systems/lp-mspm0g3507.yaml \
  --out-dir examples/lp-mspm0g3507-launchpad/evidence \
  --firmware-build-script examples/lp-mspm0g3507-launchpad/ci/build.sh
