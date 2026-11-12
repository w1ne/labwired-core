#!/bin/bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

# Script to install git hooks.

set -euo pipefail

HOOKS_DIR=$(git rev-parse --path-format=absolute --git-path hooks)
SCRIPTS_DIR=$(git rev-parse --show-toplevel)/scripts

# Do not replace an operator's custom hook. Recognize only this installer's old
# relative symlinks and its marked dispatchers, and preflight both before edits.
for hook in pre-commit pre-push; do
  installed="$HOOKS_DIR/$hook"
  if [[ -L "$installed" ]]; then
    if [[ "$(readlink "$installed")" != "../../scripts/$hook.sh" ]]; then
      echo "Refusing to replace custom hook: $installed" >&2
      exit 1
    fi
  elif [[ -e "$installed" ]] && ! grep -Fxq \
      '# LabWired repository hook dispatcher; installed by scripts/install-hooks.sh.' "$installed"; then
    echo "Refusing to replace custom hook: $installed" >&2
    exit 1
  fi
done

echo "Making scripts executable..."
chmod +x "$SCRIPTS_DIR/pre-commit.sh"
chmod +x "$SCRIPTS_DIR/pre-push.sh"

# Linked worktrees share the hook directory, but must validate their own source.
# Rename a regular file over each old symlink so installation cannot follow the
# symlink and overwrite another checkout's canonical hook script.
mkdir -p "$HOOKS_DIR"
for hook in pre-commit pre-push; do
  echo "Installing $hook hook..."
  hook_file=$(mktemp "$HOOKS_DIR/$hook.XXXXXX")
  cat > "$hook_file" <<'HOOK'
#!/bin/bash
# LabWired repository hook dispatcher; installed by scripts/install-hooks.sh.
set -euo pipefail
repo_root=$(git rev-parse --show-toplevel)
hook_name=$(basename "$0")
exec "$repo_root/scripts/$hook_name.sh" "$@"
HOOK
  chmod +x "$hook_file"
  mv -f "$hook_file" "$HOOKS_DIR/$hook"
done

echo "Hooks installed successfully."
