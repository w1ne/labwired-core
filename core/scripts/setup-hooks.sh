#!/bin/sh
#
# Helper script to install git hooks.
#

HOOKS_DIR=".git/hooks"
REPO_ROOT=$(git rev-parse --show-toplevel)

if [ ! -d "$REPO_ROOT/.git" ]; then
    echo "Error: Not a git repository."
    exit 1
fi

cp "$REPO_ROOT/scripts/pre-commit" "$REPO_ROOT/$HOOKS_DIR/pre-commit"
chmod +x "$REPO_ROOT/$HOOKS_DIR/pre-commit"

echo "Success: Pre-commit hook installed!"
