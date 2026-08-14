#!/usr/bin/env bash
# Tests for scripts/install.sh — the file every `curl … | sh` executes.
#
# They are fast and touch the network only to prove a missing archive 404s, so
# they can run on every PR. They cover
# the three ways the installer failed a user in August 2026:
#   1. it resolved "latest" to a release that carries no CLI archive,
#   2. it treated a missing archive as consent to install a Rust toolchain,
#   3. it reported success for a binary that could not start.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$repo_root/scripts/install.sh"
failures=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

check() {
  local description=$1
  shift
  if "$@"; then
    printf 'ok   %s\n' "$description"
  else
    printf 'FAIL %s\n' "$description" >&2
    failures=$((failures + 1))
  fi
}

# ── 1. Syntax, in the shells the published command can land in ────────────────
check "parses as POSIX sh" sh -n "$script"
if command -v dash >/dev/null 2>&1; then
  check "parses as dash" dash -n "$script"
fi
if command -v bash >/dev/null 2>&1; then
  check "parses as bash" bash -n "$script"
fi

# ── 2. "latest" means the newest CLI release, not the newest release ──────────
# This fixture is the shape that broke it: an asset-only demo-firmware release
# published after the last CLI release, so it sorts first in the API response.
cat > "$tmp/releases.json" <<'JSON'
[
  { "tag_name": "firmware-demos-v3", "prerelease": false },
  { "tag_name": "firmware-demos-v2", "prerelease": false },
  { "tag_name": "v0.21.0", "prerelease": false },
  { "tag_name": "v0.20.0", "prerelease": false }
]
JSON

selector() {
  # Kept character-identical to resolve_version() in install.sh.
  sed -n 's/.*"tag_name": *"\(v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' | head -1
}

selected="$(selector < "$tmp/releases.json")"
check "picks v0.21.0 over a newer firmware-demos release (got '$selected')" \
  test "$selected" = "v0.21.0"

check "the selector in install.sh is the one under test here" \
  grep -Fq 's/.*"tag_name": *"\(v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' "$script"

no_release="$(printf '[{"tag_name":"firmware-demos-v3"}]' | selector || true)"
check "an API response with no CLI release selects nothing (got '$no_release')" \
  test -z "$no_release"

# ── 3. A non-release tag is refused before any network call ──────────────────
set +e
out="$(LABWIRED_VERSION=firmware-demos-v3 LABWIRED_TELEMETRY=0 LABWIRED_NO_MODIFY_PATH=1 \
        LABWIRED_INSTALL_DIR="$tmp/bin" sh "$script" 2>&1)"
code=$?
set -e
check "a non-release version pin exits non-zero" test "$code" -ne 0
check "a non-release version pin says what a valid pin looks like" \
  grep -q "must be a release tag" <<<"$out"

# ── 4. A missing archive never installs a toolchain on its own ───────────────
set +e
out="$(LABWIRED_VERSION=v0.0.0 LABWIRED_TELEMETRY=0 LABWIRED_NO_MODIFY_PATH=1 \
        LABWIRED_INSTALL_DIR="$tmp/bin" sh "$script" 2>&1)"
code=$?
set -e
check "a missing archive exits non-zero" test "$code" -ne 0
check "a missing archive asks before building from source" \
  grep -q "LABWIRED_FROM_SOURCE=1" <<<"$out"
check "a missing archive does not install rustup behind the user's back" \
  bash -c '! grep -qi "installing via rustup" <<<"$0"' "$out"

# ── 5. Verification runs the binary, and reports what it saw ────────────────
check "the installer runs the binary it installed" grep -q 'verify_install' "$script"
check "a binary that cannot start is a failed install" \
  grep -q 'die glibc_missing' "$script"

if [ "$failures" -ne 0 ]; then
  printf '\ninstall.sh contract failed with %d issue(s).\n' "$failures" >&2
  exit 1
fi
printf '\ninstall.sh contract passed.\n'
