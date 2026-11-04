#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
build_dir="${RUNNER_TEMP:-/tmp}/labwired-nrf54l15-dk-build"

(cd "${repo_root}" && \
  make --no-print-directory -C examples/nrf54l15-dk BUILD="${build_dir}" clean && \
  make --no-print-directory -C examples/nrf54l15-dk BUILD="${build_dir}" publish)
