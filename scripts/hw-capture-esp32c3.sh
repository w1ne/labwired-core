#!/usr/bin/env bash
# Capture ESP32-C3 silicon CLOCK RATES for LabWired sim validation.
#
# Run this on a bench with an ESP32-C3 connected via its BUILT-IN USB-Serial-JTAG
# (Espressif VID 0x303A). Non-destructive: it only halts the CPU, reads MMIO over
# JTAG, and resumes — it never flashes or erases the board's firmware.
#
# It measures the two free-running timebases the C3 peripheral models depend on
# and diffs their real rates against the model's expected values:
#   * SYSTIMER  (0x6002_3040/44)  — expected 16.0 MHz  (Systimer model)
#   * RTC_CNTL  (0x6000_8010/14)  — expected RTC_SLOW_HZ_MEASURED (~136.7 kHz,
#                                    the RC_SLOW oscillator; see rtc_timer.rs)
# SYSTIMER (a hardware-fixed 16 MHz) is used as the timebase so the RTC rate is
# independent of wall-clock / JTAG-halt overhead.
#
# THIS IS NOT A PR CHECK. Hosted CI has no hardware. Run it on the bench (or a
# self-hosted `hil`+`esp32c3` runner / the weekly core-validate-hw-targets job),
# then record the dated result in validation/manifest.yaml (the honesty ledger).
#
# Prereq: openocd-esp32 (Espressif fork — vanilla openocd 0.12 has no C3 target).
#   xPack:  npm i -g @xpack-dev-tools/openocd  (or the esp-idf openocd)
#   Release: https://github.com/espressif/openocd-esp32/releases
#
# Usage:
#   bash core/scripts/hw-capture-esp32c3.sh
#
# Exit 0 if both rates are within tolerance of the model; non-zero otherwise.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${REPO_ROOT}/core/fixtures/esp32c3/hw-capture-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

# ── Locate openocd-esp32 (must support the C3 RISC-V target) ─────────────────
# Override with OPENOCD_ESP32=/path/to/openocd (Espressif fork). Candidates are
# real-path resolved so a symlink on PATH still points us at its install tree
# (…/bin/openocd → …/share/openocd/scripts).
OOCD=""
for cand in \
  "${OPENOCD_ESP32:-}" \
  "$(command -v openocd-esp32 2>/dev/null || true)" \
  "$HOME/.local/openocd-esp32/bin/openocd" \
  "$HOME/.espressif/tools/openocd-esp32"/*/openocd-esp32/bin/openocd \
  "$HOME/.local/xpack-openocd/bin/openocd" \
  "$(command -v openocd 2>/dev/null || true)"; do
  [ -n "$cand" ] && [ -x "$cand" ] || continue
  real="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$cand")"
  scr="$(dirname "$real")/../share/openocd/scripts"
  if [ -f "$scr/board/esp32c3-builtin.cfg" ]; then OOCD="$real"; OOCD_SCR="$scr"; break; fi
done
if [ -z "$OOCD" ]; then
  echo "::error::openocd-esp32 with board/esp32c3-builtin.cfg not found."
  echo "Install the Espressif openocd fork (vanilla openocd has no C3 target):"
  echo "  https://github.com/espressif/openocd-esp32/releases"
  exit 1
fi
echo "==> openocd: $("$OOCD" --version 2>&1 | head -1)"
echo "==> scripts: $OOCD_SCR"
echo "==> output:  $OUT"

# ── Capture two SYSTIMER+RTC samples ~2 s apart with the CPU running between ──
# `capture {mdw ...}` is required on openocd-esp32 to pull the read value into a
# Tcl variable (the raw mdw log line otherwise goes to a channel we can't $-expand).
"$OOCD" -s "$OOCD_SCR" -f board/esp32c3-builtin.cfg -c '
proc snap {label} {
  mww 0x60023004 0x40000000
  set sl [capture {mdw 0x60023044}]
  mww 0x6000800C 0x80000000
  set rl [capture {mdw 0x60008010}]
  echo "SNAP $label SYS=$sl RTC=$rl"
}
init
halt
snap A
resume
sleep 2000
halt
snap B
resume
exit
' 2>&1 | tee "$OUT/openocd.log" | grep -E "^SNAP|Info : \[esp32c3\]|Chip revision" || true

# ── Reduce to rates and assert against the model ─────────────────────────────
python3 - "$OUT/openocd.log" "$OUT/result.txt" <<'PY'
import re, sys
log, out = sys.argv[1], sys.argv[2]
txt = open(log).read()
def val(label, kind):
    m = re.search(rf"SNAP {label} .*{kind}=0x[0-9a-fA-F]+: ([0-9a-fA-F]+)", txt)
    if not m: sys.exit(f"ERROR: no {kind} reading for sample {label}\n{txt}")
    return int(m.group(1), 16)
sysA, sysB = val("A","SYS"), val("B","SYS")
rtcA, rtcB = val("A","RTC"), val("B","RTC")
dsys, drtc = sysB - sysA, rtcB - rtcA
if dsys <= 0: sys.exit(f"ERROR: SYSTIMER did not advance ({sysA}->{sysB})")

SYS_HZ = 16_000_000                 # model: Systimer 16 MHz
RTC_HZ_MODEL = 136_700             # model: RTC_SLOW_HZ_MEASURED
elapsed = dsys / SYS_HZ            # SYSTIMER as the timebase
sys_rate = dsys / 2.0             # wall-clock ~2 s sanity
rtc_rate = drtc / elapsed
lines = [
    f"SYSTIMER: delta={dsys} ticks  ~{sys_rate/1e6:.3f} MHz (wall) / model 16.000 MHz",
    f"RTC:      delta={drtc} ticks  {rtc_rate/1e3:.2f} kHz (SYSTIMER-referenced) / model {RTC_HZ_MODEL/1e3:.1f} kHz",
    f"elapsed (16 MHz base): {elapsed:.4f} s",
]
# Tolerances: SYSTIMER within 2% of 16 MHz; RTC within 5% of the model value
# (RC_SLOW is an uncalibrated RC oscillator — a few % board-to-board is expected).
ok_sys = abs(sys_rate - SYS_HZ) / SYS_HZ < 0.02
ok_rtc = abs(rtc_rate - RTC_HZ_MODEL) / RTC_HZ_MODEL < 0.05
verdict = "PASS" if (ok_sys and ok_rtc) else "FAIL"
lines += [f"SYSTIMER {'ok' if ok_sys else 'OUT-OF-TOLERANCE'}, RTC {'ok' if ok_rtc else 'OUT-OF-TOLERANCE'} -> {verdict}"]
report = "\n".join(lines)
open(out, "w").write(report + "\n")
print(report)
sys.exit(0 if verdict == "PASS" else 2)
PY
rc=$?
echo "==> result: $OUT/result.txt (exit $rc)"
exit $rc
