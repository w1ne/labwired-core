#!/bin/bash
# Ryan's twelve, end to end. One line per requirement; nothing hidden.
#
# Tests 10 and 11 also need DISPLAY evidence. The CLI test script has no
# `display` clause (that one is MCP-only), so the runner reads painted bytes
# out of result.json's `inspect` block — the panel model's own framebuffer,
# not a "driver was initialised" flag.
LW=${LW:-/Volumes/LabWired/cargo-target/release/labwired}
OUT=${OUT:-./.out}
mkdir -p "$OUT"

run_script() {   # $1 = script
  d="$OUT/$(basename "$1" .yaml)"
  [ -f "$d/.done" ] && return
  "$LW" test --script "$1" --no-uart-stdout --output-dir "$d" >/dev/null 2>&1
  touch "$d/.done"
}
status_of() { python3 -c "import json;print(json.load(open('$1/result.json'))['status'])" 2>/dev/null || echo ERROR; }
# Display evidence comes from `snapshot capture`, NOT from the test runner.
# `labwired test`'s result.json `inspect` block carries ZERO artifacts — no
# panel, no framebuffer — and the script schema has no `display` clause either
# (that one is MCP-only). So the declarative runner cannot assert display
# output at all today. Ryan's tests 10 and 11 need it, so the check runs the
# same firmware + system through snapshot, which does expose the panel's own
# painted-byte count.
panel_ink() {   # $1 = system yaml
  "$LW" snapshot capture \
    --firmware ../firmware/.pio/build/adafruit_feather_esp32_v2/firmware.elf \
    --system "$1" --steps 120000000 --progress-every 0 \
    --output "$OUT/panel.lwrs" 2>&1 \
  | sed -n 's/.*painted bytes=\([0-9]*\).*/\1/p' | head -1
}

report() {  # num, label, script, min_ink
  d="$OUT/$(basename "$3" .yaml)"
  st=$(status_of "$d"); note=""
  if [ "$4" != "0" ]; then
    got=$(panel_ink "$5"); [ -z "$got" ] && got=0; note="  (panel ink=$got)"
    if [ "$got" -lt "$4" ]; then st="fail-no-paint"; fi
  fi
  if [ "$st" = "pass" ]; then printf '  PASS  %-7s %s%s\n' "$1" "$2" "$note"
  else printf '  FAIL  %-7s %s%s  [%s]\n' "$1" "$2" "$note" "$st"; FAILED=1; fi
}

rm -f "$OUT"/*/.done 2>/dev/null
FAILED=0
for s in test-thresholds-hysteresis.yaml test-debounce.yaml test-occupancy-combinations.yaml test-fault-and-display.yaml test-nonblocking.yaml; do run_script "$s"; done

report "1,2,3" "init, per-read channel select, independent injection" test-thresholds-hysteresis.yaml 0
report "4"     "raw counts -> EMPTY/PRESENT via thresholds"           test-thresholds-hysteresis.yaml 0
report "5"     "separate entry/exit thresholds (hysteresis)"          test-thresholds-hysteresis.yaml 0
report "6"     "debounce: single noisy spike rejected"                test-debounce.yaml 0
report "7"     "simultaneous occupancy across all four bays"          test-occupancy-combinations.yaml 0
report "8"     "one channel cannot change another's state"            test-occupancy-combinations.yaml 0
report "9"     "missing sensor / I2C failure detected"                test-fault-and-display.yaml 0
report "10"    "per-bay state painted on the TFT"                     test-fault-and-display.yaml 500 ../system-sensor-missing.yaml
report "11"    "clear FAULT state for an unreadable sensor"           test-fault-and-display.yaml 500 ../system-sensor-missing.yaml
report "12"    "polling and display do not block each other"          test-nonblocking.yaml 500 ../system.yaml
exit $FAILED
