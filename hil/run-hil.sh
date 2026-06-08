#!/usr/bin/env bash
# LabWired - HIL oracle runner.
#
# Runs the silicon-anchored oracle bank for one (or all active) HIL board(s)
# described in hil/boards.json. Designed to run on a self-hosted runner that has
# the board attached, but also works standalone on a dev bench with the board
# plugged in — that's how you smoke-test the harness before deploying a runner.
#
# Usage:
#   hil/run-hil.sh <board-id|all>
#
# Requires: cargo, openocd on PATH (the script adds the common Homebrew and
# xpack install locations defensively), python3, and the probe attached.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
manifest="$here/boards.json"
target="${1:-all}"

# openocd is rarely on PATH by default; add the usual suspects.
export PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/xpack-openocd/bin:$PATH"
command -v openocd >/dev/null 2>&1 || {
  echo "::error::openocd not found on PATH (looked in Homebrew + ~/.local/xpack-openocd/bin)"
  exit 1
}
echo "openocd: $(openocd --version 2>&1 | head -1)"

# Emit a tab-separated row per active board to run: id, test, features, env-json.
boards="$(python3 - "$manifest" "$target" <<'PY'
import json, sys
manifest, target = sys.argv[1], sys.argv[2]
data = json.load(open(manifest))
for b in data["boards"]:
    if b.get("status") != "active":
        continue
    if target not in ("all", b["id"]):
        continue
    print("\t".join([b["id"], b["test"], b.get("features", ""), json.dumps(b.get("env", {}))]))
PY
)"

if [ -z "$boards" ]; then
  echo "::warning::no active board matched '$target' in $manifest"
  exit 0
fi

fail=0
while IFS=$'\t' read -r id test features env_json; do
  [ -z "$id" ] && continue
  echo "::group::HIL $id ($test)"
  # Export the board's env (e.g. STM32_TARGET).
  while IFS=$'\t' read -r k v; do
    [ -z "$k" ] && continue
    export "$k=$v"
    echo "env: $k=$v"
  done < <(python3 -c 'import json,sys; [print(f"{k}\t{v}") for k,v in json.loads(sys.argv[1]).items()]' "$env_json")

  feat_args=()
  [ -n "$features" ] && feat_args=(--features "$features")

  # The oracle _hw/_diff tests are #[ignore]; --test-threads=1 serialises the
  # single physical board.
  if cargo test --manifest-path "$root/Cargo.toml" \
       -p labwired-hw-oracle --test "$test" "${feat_args[@]}" \
       -- --ignored --test-threads=1; then
    echo "HIL $id: PASS"
  else
    echo "::error::HIL $id: FAIL"
    fail=1
  fi
  echo "::endgroup::"
done <<< "$boards"

exit "$fail"
