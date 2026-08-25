#!/usr/bin/env bash
# Uniform hardware capture for the chip conformance standard.
#
#   hw_conform.sh <target.json> <out_dir>
#
# Reads a per-chip target descriptor (transport + register windows), captures the
# reset-state register words from a CONNECTED board over its debug transport, and
# writes a reg_oracle.json in the standard schema. Commit that oracle and the
# chip_conformance ratchet will diff the sim against it (raising the chip's level).
#
# Currently implements the openocd-esp32 transport (ESP32-* USB-JTAG). Other
# transports (ST-Link SWD for STM32, CMSIS-DAP) slot in as new `tool` branches.
set -uo pipefail
TARGET="${1:?usage: hw_conform.sh <target.json> <out_dir>}"
OUT="${2:?need out dir}"; mkdir -p "$OUT"

chip=$(python3 -c "import json,sys;print(json.load(open('$TARGET'))['chip'])")
schema=$(python3 -c "import json,sys;print(json.load(open('$TARGET'))['schema'])")
tool=$(python3 -c "import json,sys;print(json.load(open('$TARGET'))['transport']['tool'])")

case "$tool" in
  openocd-esp32)
    home=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport']['openocd_home'])")
    board=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport']['board_cfg'])")
    speed=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport'].get('adapter_speed',4000))")
    tmo=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport'].get('command_timeout_sec',10))")
    # Build the openocd command list. mdw output only reaches the log via tcl
    # `capture {}` in -c batch mode — do not remove the capture wrapper.
    cmds="adapter speed $speed; riscv set_command_timeout_sec $tmo; init; reset halt;"
    while read -r id base count; do
      cmds+=" echo {@@$id $base}; echo [capture {mdw $base $count}];"
    done < <(python3 -c "import json
for w in json.load(open('$TARGET'))['windows']: print(w['id'], w['base'], w['count'])")
    cmds+=" exit"
    for attempt in 1 2 3 4; do
      "$home/bin/openocd" -s "$home/share/openocd/scripts" -f "$board" -c "$cmds" > "$OUT/openocd.log" 2>&1
      grep -q ": " "$OUT/openocd.log" && grep -qE "^0x" "$OUT/openocd.log" && break
      echo "capture attempt $attempt: examination flaky, retrying"; sleep 1
    done
    ;;
  openocd-jlink)
    # SEGGER J-Link OB over SWD — what every Silicon Labs kit carries. Same
    # openocd invocation as the ST-Link branch; the only difference is that SWD
    # must be selected explicitly BEFORE the target cfg is sourced, because
    # jlink.cfg defaults to JTAG and efm32.cfg has no TAP to find there.
    #
    # ⚠️ probe-rs is NOT used here even though the rest of this board's tooling
    # is probe-rs. Its CLI has no reset-halt, and `--connect-under-reset` times
    # out on BRD2709A ("target is not responding to the reset sequence") — the
    # probe's nRESET is not wired through. Without a halt at reset the capture
    # reads whatever firmware left behind, which is not a reset-state oracle:
    # measured on this board, CLKEN0/1/2 read c4010010/16014bff/00000080 with
    # the BLE beacon running and 00000000 x3 under openocd `reset halt`.
    oocd=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport'].get('openocd','openocd'))")
    icfg=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport']['interface_cfg'])")
    tcfg=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport']['target_cfg'])")
    tsel=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport'].get('transport_select','swd'))")
    cmds="init; reset halt;"
    # OPTIONAL PREAMBLE, and on this chip it is not optional in practice.
    # A Series-2 peripheral that is not clocked does not read as zero over the
    # debug port — it FAULTS, and openocd abandons the rest of the command list
    # after "Failed to read memory at ...". Measured: a bare `reset halt`
    # capture on BRD2709A returns the CMU window and then dies at GPIO.
    # So the preamble un-gates the peripherals this oracle reads, and nothing
    # else: every register captured below is still its reset value.
    while read -r addr val; do
      cmds+=" mww $addr $val;"
    done < <(python3 -c "import json
for s in json.load(open('$TARGET')).get('preamble', []):
    print(s['write32']['address'], s['write32']['value'])")
    while read -r id base count; do
      cmds+=" echo {@@$id $base}; echo [capture {mdw $base $count}];"
    done < <(python3 -c "import json
for w in json.load(open('$TARGET'))['windows']: print(w['id'], w['base'], w['count'])")
    cmds+=" exit"
    "$oocd" -f "$icfg" -c "transport select $tsel" -f "$tcfg" -c "$cmds" > "$OUT/openocd.log" 2>&1
    ;;
  openocd-stlink)
    oocd=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport'].get('openocd','openocd'))")
    icfg=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport']['interface_cfg'])")
    tcfg=$(python3 -c "import json;print(json.load(open('$TARGET'))['transport']['target_cfg'])")
    cmds="init; reset halt;"
    while read -r id base count; do
      cmds+=" echo {@@$id $base}; echo [capture {mdw $base $count}];"
    done < <(python3 -c "import json
for w in json.load(open('$TARGET'))['windows']: print(w['id'], w['base'], w['count'])")
    cmds+=" exit"
    "$oocd" -f "$icfg" -f "$tcfg" -c "$cmds" > "$OUT/openocd.log" 2>&1
    ;;
  *)
    echo "transport '$tool' not yet implemented — add a branch in hw_conform.sh"; exit 2;;
esac

# Parse the @@id markers + mdw word rows into the standard reg_oracle.json schema.
python3 - "$OUT/openocd.log" "$OUT/reg_oracle.json" "$chip" "$schema" "$TARGET" <<'PY'
import sys,re,json
log,outp,chip,schema=open(sys.argv[1]).read(),sys.argv[2],sys.argv[3],sys.argv[4]
blocks={};cur=base=None
for line in log.splitlines():
    m=re.match(r'@@(\S+)\s+(0x[0-9a-fA-F]+)',line)
    if m: cur,base=m.group(1),m.group(2); blocks[cur]={'base':base,'words':{}}; continue
    m=re.match(r'(0x[0-9a-fA-F]+):\s+(.*)',line)
    if m and cur:
        a=int(m.group(1),16)
        for i,w in enumerate(re.findall(r'[0-9a-fA-F]{8}',m.group(2))):
            blocks[cur]['words'][hex(a+4*i)]=f'0x{int(w,16):08x}'
# A capture that ran a preamble is NOT a pure reset_halt, and a file that says
# it is would be lying about how it was taken. Record the writes.
pre=json.load(open(sys.argv[5])).get('preamble', []) if len(sys.argv)>5 else []
doc={'schema':schema,'chip':chip,'state':'reset_halt','blocks':blocks}
if pre:
    doc['state']='reset_halt+preamble'
    doc['preamble']=pre
json.dump(doc,open(outp,'w'),indent=1)
n=sum(len(b['words']) for b in blocks.values())
print(f"captured {n} words across {len(blocks)} windows -> {outp}")
PY
