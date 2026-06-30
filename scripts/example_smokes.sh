#!/usr/bin/env bash
# Run every example smoke/test whose firmware ELF is available, asserting each
# demo actually works (real output, not a stub). Examples are the user-facing
# labs (web playground + CLI), and they rot silently when nothing runs them —
# this is the autocheck that keeps them honest.
#
# For each examples/<name>/{*smoke*,test*}.yaml whose referenced firmware ELF
# exists on disk, run `labwired test` and collect pass/fail. Examples whose ELF
# is NOT committed/built are reported as UNCOVERED (visible, not silent) so the
# coverage gap is obvious in the nightly summary.
#
# Exit non-zero if any runnable example fails. Uncovered examples do not fail
# the run by default (pass --strict to also fail when an example is uncovered).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STRICT=0
[ "${1:-}" = "--strict" ] && STRICT=1

CLI=(cargo run -q -p labwired-cli --)
OUT_ROOT="${OUT_DIR:-out/example-smokes}"
mkdir -p "$OUT_ROOT"

# Expected-FAIL / self-managed smokes: the f103-fidelity-bench cases run
# deliberately-broken firmware and assert the sim FAILS it (a pass would be a
# false pass). They have their own outcome-aware runner (run-benchmark.sh), so a
# naive pass/fail runner must not flag them. Matched on "<dir>/<yaml-basename>".
is_skipped() {
  case "$1" in
    f103-fidelity-bench/clockbug-smoke.yaml|\
    f103-fidelity-bench/clockbug-nogate-smoke.yaml|\
    f103-fidelity-bench/gpiobug-smoke.yaml|\
    f103-fidelity-bench/rambug-smoke.yaml) return 0 ;;
    *) return 1 ;;
  esac
}

pass=0 fail=0 uncovered=0
failed_names=()

echo "## Example smoke coverage"
echo
printf '%-30s %-10s %s\n' "EXAMPLE" "RESULT" "SCRIPT"
printf '%-30s %-10s %s\n' "-------" "------" "------"

for yaml in examples/*/*smoke*.yaml examples/*/test*.yaml; do
  [ -f "$yaml" ] || continue
  dir="$(dirname "$yaml")"
  name="$(basename "$dir")"
  if is_skipped "$name/$(basename "$yaml")"; then
    printf '%-30s %-10s %s\n' "$name" "SKIP" "$(basename "$yaml") (expected-fail; see run-benchmark.sh)"
    continue
  fi
  # firmware path is relative to the yaml's directory
  fw_rel="$(grep -E '^\s*firmware:' "$yaml" | head -1 | sed -E 's/.*firmware:[[:space:]]*"?([^"]*)"?.*/\1/')"
  fw_path="$dir/${fw_rel#./}"
  if [ ! -f "$fw_path" ]; then
    printf '%-30s %-10s %s\n' "$name" "UNCOVERED" "$(basename "$yaml") (no ELF: $fw_rel)"
    uncovered=$((uncovered + 1))
    continue
  fi
  od="$OUT_ROOT/$name-$(basename "$yaml" .yaml)"
  if "${CLI[@]}" test --script "$yaml" --output-dir "$od" --no-uart-stdout >/dev/null 2>&1; then
    printf '%-30s %-10s %s\n' "$name" "PASS" "$(basename "$yaml")"
    pass=$((pass + 1))
  else
    printf '%-30s %-10s %s\n' "$name" "FAIL" "$(basename "$yaml")"
    fail=$((fail + 1))
    failed_names+=("$name/$(basename "$yaml")")
  fi
done

echo
echo "Passed: $pass   Failed: $fail   Uncovered (no committed ELF): $uncovered"
if [ "$fail" -gt 0 ]; then
  echo "FAILED: ${failed_names[*]}"
  exit 1
fi
if [ "$STRICT" -eq 1 ] && [ "$uncovered" -gt 0 ]; then
  echo "STRICT: $uncovered example(s) have no committed ELF and were not run."
  exit 2
fi
exit 0
