#!/bin/bash
#
# Helper script to install git hooks.
#

set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)

if [ ! -d "$REPO_ROOT/.git" ]; then
  echo "Error: Not a git repository."
  exit 1
fi

"$REPO_ROOT/scripts/install-hooks.sh"

echo "Success: pre-commit and pre-push hooks installed."
