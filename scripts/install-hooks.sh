#!/bin/bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

# Script to install git hooks.

set -euo pipefail

HOOKS_DIR=$(git rev-parse --show-toplevel)/.git/hooks
SCRIPTS_DIR=$(git rev-parse --show-toplevel)/scripts

echo "Making scripts executable..."
chmod +x "$SCRIPTS_DIR/pre-commit.sh"
chmod +x "$SCRIPTS_DIR/pre-push.sh"

echo "Installing pre-commit hook..."
ln -sf "../../scripts/pre-commit.sh" "$HOOKS_DIR/pre-commit"

echo "Installing pre-push hook..."
ln -sf "../../scripts/pre-push.sh" "$HOOKS_DIR/pre-push"

echo "Hooks installed successfully."
