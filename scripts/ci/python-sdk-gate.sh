#!/usr/bin/env bash
# Build the labwired Python wheel, install it into a clean venv, and run
# crates/python/tests against the INSTALLED package.
#
# The gate fails when:
#   * the wheel does not build or install,
#   * `import labwired` fails or resolves outside the venv's site-packages,
#   * any test fails or errors,
#   * any test is SKIPPED, or no test ran at all.
# A fully skipped suite exits 0 in pytest; the JUnit count below turns that
# into a red job.
#
# Usage: scripts/ci/python-sdk-gate.sh [work-dir]
#        scripts/ci/python-sdk-gate.sh --check-sdist <labwired-X.Y.Z.tar.gz>
# Needs: python3 (>= 3.9) with venv, and a Rust toolchain on PATH.
set -euo pipefail

# An sdist without the staged chip catalog packs fine and then fails to
# build on the user's machine (crates/config/build.rs needs configs/chips).
if [ "${1:-}" = "--check-sdist" ]; then
  sdist="${2:?usage: --check-sdist <sdist.tar.gz>}"
  listing="$(tar tzf "$sdist")"
  for dir in configs/chips python/labwired/configs/chips; do
    if ! grep -Eq "^[^/]+/${dir}/[^/]+\.yaml\$" <<<"$listing"; then
      echo "::error::$sdist has no ${dir}/*.yaml" >&2
      exit 1
    fi
  done
  echo "sdist ok: $sdist carries configs/chips and python/labwired/configs/chips"
  exit 0
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
work="${1:-$(mktemp -d)}"
mkdir -p "$work"
work="$(cd "$work" && pwd)"
python="${PYTHON:-python3}"

echo "== build venv: $work/build-venv"
"$python" -m venv "$work/build-venv"
"$work/build-venv/bin/python" -m pip install --quiet --upgrade pip 'maturin>=1.5,<2'

echo "== maturin build"
rm -rf "$work/dist"
"$work/build-venv/bin/maturin" build --release \
  --manifest-path "$repo/crates/python/Cargo.toml" --out "$work/dist"
shopt -s nullglob
wheels=("$work"/dist/labwired-*.whl)
shopt -u nullglob
if [ "${#wheels[@]}" -ne 1 ]; then
  echo "::error::expected exactly one wheel in $work/dist, found ${#wheels[@]}" >&2
  exit 1
fi
case "${wheels[0]}" in
  *-abi3-*) ;;
  *) echo "::error::wheel is not abi3: ${wheels[0]}" >&2; exit 1 ;;
esac
echo "wheel: ${wheels[0]}"

echo "== clean test venv: $work/test-venv"
rm -rf "$work/test-venv"
"$python" -m venv "$work/test-venv"
venv_py="$work/test-venv/bin/python"
"$venv_py" -m pip install --quiet "${wheels[0]}" 'pytest>=7'

# Run from outside the checkout, so `import labwired` cannot pick up the
# source tree (crates/python/python) instead of the installed wheel.
cd "$work"
"$venv_py" - <<'PY'
import labwired, sys
assert "site-packages" in labwired.__file__, labwired.__file__
assert hasattr(labwired.Sim, "read_uart_bytes"), "read_uart_bytes missing"
print("import ok:", labwired.__file__, "on", sys.version.split()[0])
PY

echo "== pytest"
junit="$work/junit.xml"
rm -f "$junit"
status=0
"$venv_py" -m pytest -p no:cacheprovider -rs -v \
  --junitxml "$junit" "$repo/crates/python/tests" || status=$?

"$venv_py" - "$junit" <<'PY'
import sys
import xml.etree.ElementTree as ET

root = ET.parse(sys.argv[1]).getroot()
suites = [root] if root.tag == "testsuite" else list(root.iter("testsuite"))
total = sum(int(s.get("tests", 0)) for s in suites)
skipped = sum(int(s.get("skipped", 0)) for s in suites)
failed = sum(int(s.get("failures", 0)) + int(s.get("errors", 0)) for s in suites)
print(f"junit: tests={total} failed+errors={failed} skipped={skipped}")
if total == 0:
    sys.exit("::error::no Python SDK test ran")
if skipped:
    sys.exit(f"::error::{skipped} Python SDK test(s) skipped; this gate requires 0")
PY

if [ "$status" -ne 0 ]; then
  echo "::error::pytest exited $status" >&2
  exit "$status"
fi
echo "== python SDK gate: PASS"
