#!/usr/bin/env bash
# Poll-surface trace of the WiFi-init probe on a live ESP32-C3.
#
# trace_radio.sh recovered the *write* surface (which radio regs libphy/the MAC
# driver configure) by diffing full-window dumps at coarse phase anchors. This
# script recovers the complementary *poll* surface: the MAC status bits the
# driver busy-waits on inside esp_wifi_start(). A phase snapshot can't see those
# — they resolve between breakpoints — so instead we arm a hardware READ
# watchpoint over the MAC window and log every load the CPU issues against it
# (PC + address + value) while single-stepping through the esp_wifi_start
# bracket. Diff the values a given address returns across consecutive reads and
# the bit that flips 0->1 right before the spin exits is the "ready" bit the
# behavioral model must flip. Feed the output to poll_surface_to_table.py.
#
# Needs: openocd-esp32, ESP-IDF v5.3.1 toolchain, a C3 on built-in USB-JTAG.
# Usage: trace_poll.sh <wifi_probe.elf> <out_dir> [mac_base] [mac_words]
set -uo pipefail
ELF="${1:?usage: trace_poll.sh <wifi_probe.elf> <out_dir> [mac_base] [mac_words]}"
OUT="${2:?need out dir}"; mkdir -p "$OUT"
# The C3 MAC window headlined in the RE doc; 0x60033000..0x60035000 (~46 regs).
MAC_BASE="${3:-0x60033000}"
MAC_WORDS="${4:-128}"   # 128 words = 0x200 bytes; covers the dense status block.
# Watchpoint access type: r=reads (poll surface, default), w=writes (command
# surface), a=both. Capturing w and r separately and correlating by PC recovers
# the driver's write-command -> poll-done handshake protocol.
WP_TYPE="${WP_TYPE:-r}"

OOCD=/private/tmp/openocd-esp32/bin/openocd
SCRIPTS=/private/tmp/openocd-esp32/share/openocd/scripts
NM=$(ls ~/.espressif/tools/riscv32-esp-elf/*/riscv32-esp-elf/bin/riscv32-esp-elf-nm 2>/dev/null | head -1)
[ -z "$NM" ] && NM=riscv32-esp-elf-nm

addr() { "$NM" "$ELF" | awk -v s="$1" '$3==s{print "0x"$1}'; }
A_ENTER=$(addr probe_start_enter)
A_DONE=$(addr probe_after_start)
[ -z "$A_ENTER" ] && { echo "FATAL: probe_start_enter not in $ELF (rebuild the probe)"; exit 2; }
echo "bracket: enter=$A_ENTER done=$A_DONE  mac=$MAC_BASE+$MAC_WORDS"

# RISC-V mcontrol read-watchpoint over the MAC window. NAPOT range needs the
# length to be a power of two and the base aligned to it; round the byte span up.
MAC_LEN=$(( MAC_WORDS * 4 ))
pow2=1; while [ "$pow2" -lt "$MAC_LEN" ]; do pow2=$(( pow2 * 2 )); done
echo "watchpoint span: $pow2 bytes (NAPOT)"

# Strategy: break at probe_start_enter, then loop: arm a read watchpoint on the
# MAC window, resume, wait for the watchpoint to halt the core, and snapshot
# pc + the whole window. The hit address is whichever word the load touched;
# logging the full window each hit lets the offline differ find the flipping
# bit without depending on openocd reporting the exact trigger address (its
# RISC-V trigger introspection is uneven across versions). Bounded by MAX_HITS
# so a tight spin loop can't run the capture forever.
MAX_HITS="${MAX_HITS:-400}"
{
  echo "adapter speed 4000; riscv set_command_timeout_sec 10; init; reset halt"
  echo "bp $A_ENTER 2 hw"
  [ -n "$A_DONE" ] && echo "bp $A_DONE 2 hw"
  echo "resume; wait_halt 30000"          # run up to probe_start_enter
  echo "echo {##BRACKET enter}"
  # Arm a fresh read watchpoint over the window. Each trace_poll.sh run is its
  # own openocd process, so there's no stale wp to remove first (a speculative
  # `rwp` would error on "no watchpoint found" and abort the whole -c batch).
  echo "wp $MAC_BASE $pow2 $WP_TYPE"         # watchpoint over the window (r/w/a)
  echo "echo {##DONE_ADDR $A_DONE}"
  # One TCL for-loop emitted as a single line: bash-unrolling would put `break`
  # outside any TCL loop (error), and `tr '\n' ';'` would shatter a multi-line
  # for{}{}{}{} into bad syntax. \$i stays literal for TCL; catch{} lets a
  # wait_halt timeout (spin stopped touching MAC) end the capture cleanly.
  echo "for {set i 1} {\$i <= $MAX_HITS} {incr i} {if {[catch {resume; wait_halt 10000}]} {echo {##HALT_TIMEOUT}; break}; echo \"##HIT \$i\"; echo [capture {reg pc}]; echo [capture {mdw $MAC_BASE $MAC_WORDS}]}"
  echo "catch {rwp $MAC_BASE}"
  echo "echo {##BRACKET done}"
  echo "exit"
} | tr '\n' ';' > "$OUT/poll_cmds.tcl"

"$OOCD" -s "$SCRIPTS" -f board/esp32c3-builtin.cfg -c "$(cat "$OUT/poll_cmds.tcl")" > "$OUT/poll_trace.log" 2>&1
rc=$?
echo "openocd exit $rc  -> $OUT/poll_trace.log"
hits=$(grep -c "##HIT" "$OUT/poll_trace.log" 2>/dev/null || echo 0)
echo "watchpoint hits captured: $hits"
[ "$hits" = 0 ] && echo "WARN: 0 hits — driver may poll a different window; widen MAC_BASE/MAC_WORDS or check the watchpoint armed (grep 'Watchpoint' $OUT/poll_trace.log)."
echo "next: python3 $(dirname "$0")/poll_surface_to_table.py $OUT/poll_trace.log"
