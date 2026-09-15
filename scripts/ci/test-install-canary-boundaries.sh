#!/usr/bin/env bash
# Keep labwired-core's canary focused on surfaces this repository owns.
set -euo pipefail

workflow=${1:-.github/workflows/install-canary.yml}

test -f "$workflow"
! rg -q 'marketplace\.visualstudio\.com|open-vsx\.org|labwired-vscode|extension is installable' "$workflow"
