#!/usr/bin/env bash
# Phase-by-phase radio MMIO trace of the WiFi-init probe on a live ESP32-C3.
# Breaks at each probe anchor and dumps candidate radio windows; diff the phases
# to recover the register surface the PHY/MAC driver configures.
set -uo pipefail
ELF="${1:?usage: trace_radio.sh <wifi_probe.elf> <out_dir>}"
OUT="${2:?need out dir}"; mkdir -p "$OUT"
OOCD=/private/tmp/openocd-esp32/bin/openocd
SCRIPTS=/private/tmp/openocd-esp32/share/openocd/scripts
NM=$(ls ~/.espressif/tools/riscv32-esp-elf/*/riscv32-esp-elf/bin/riscv32-esp-elf-nm 2>/dev/null | head -1)
[ -z "$NM" ] && NM=riscv32-esp-elf-nm

addr() { "$NM" "$ELF" | awk -v s="$1" '$3==s{print "0x"$1}'; }
A_BEFORE=$(addr probe_before_init); A_AINIT=$(addr probe_after_init)
A_ASTART=$(addr probe_after_start); A_IDLE=$(addr probe_idle)
echo "anchors: before=$A_BEFORE after_init=$A_AINIT after_start=$A_ASTART idle=$A_IDLE"

# candidate radio windows (base count) — front-end, baseband, MAC-gap scan
WINS="0x60005000:16 0x60006000:16 0x6001cc00:64 0x6001d000:64 0x6001e000:64 \
0x60021000:16 0x60022000:16 0x60027000:64 0x60028000:64 0x60029000:64 0x6002a000:64 \
0x60030000:64 0x60031000:64 0x60032000:64 0x60033000:64 0x60034000:64 0x60035000:64 \
0x60036000:64 0x60037000:64 0x60038000:64 0x60039000:64 0x60041000:64 0x60042000:64"

dumpcmds() { local tag="$1"; echo "echo {##PHASE $tag}"; for w in $WINS; do b=${w%:*}; n=${w#*:}; echo "echo {@@$b}"; echo "echo [capture {mdw $b $n}]"; done; }

{
  echo "adapter speed 4000; riscv set_command_timeout_sec 10; init; reset halt"
  for bp in "$A_BEFORE" "$A_AINIT" "$A_ASTART" "$A_IDLE"; do echo "bp $bp 2 hw"; done
  echo "resume; wait_halt 15000"; dumpcmds before_init
  echo "resume; wait_halt 15000"; dumpcmds after_init
  echo "resume; wait_halt 15000"; dumpcmds after_start
  echo "resume; wait_halt 15000"; dumpcmds idle
  echo "exit"
} | tr '\n' ';' > "$OUT/cmds.tcl"

"$OOCD" -s "$SCRIPTS" -f board/esp32c3-builtin.cfg -c "$(cat "$OUT/cmds.tcl")" > "$OUT/trace.log" 2>&1
echo "openocd exit $?  -> $OUT/trace.log"
grep -c "##PHASE" "$OUT/trace.log" | sed 's/^/phases captured: /'
