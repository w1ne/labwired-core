#!/usr/bin/env bash
# Boots the real Raspberry Pi OS arm64 kernel in QEMU raspi4b. Every BSC
# (I2C controller) the device tree enables is served by the LabWired BSC model
# over the bridge; addresses and IRQs come from the DTB through
# `qemu-bridge plan`, nothing is hardcoded here. Checks run from the guest
# console and the bridge transcripts.
#
# Runs (guest = kernel8.img + an initramfs built from the image's busybox and
# i2c modules; -icount unless noted):
#   positive     i2c1 + BME280@0x76        -> 0x76 present, i2cget 0xd0 = 0x60
#   positive-2   same again                -> transcript byte-identical (cmp)
#   no-device    i2c1, no I2C target       -> 0x76 absent
#   no-bridge    QEMU's own BSC model      -> 0x76 absent
#   host-1/2     positive without -icount  -> transcripts differ (proves the
#                determinism check can fail)
#   dual         i2c1 + i2c3 both enabled, BME280@0x76 on i2c1, BME280@0x77 on
#                i2c3, concurrent i2cget loops on both buses (shared SPI 117)
#   dual-2       same again                -> both transcripts byte-identical
#   dual-noor    same with the bridge IRQ OR off (irq-or=off): negative control
#   disabled     i2c1 disabled in the DTB  -> loader makes no bridge, no 0x76
# Plus a timing check of the i2cget transfer against the SCL formula.
#
# Needs: qemu/build.sh done (or it runs it), cargo, dtc, mtools, e2fsprogs,
# python3, xz.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
ROOT="${QEMU_LW_ROOT:-/Volumes/LabWired/caches/qemu-labwired}"
IMAGES="$ROOT/images"
WORK="$ROOT/work"
LOGS="${LOGS:-$ROOT/logs}"
QEMU="$ROOT/build/qemu-system-aarch64"
BOOT_TIMEOUT="${BOOT_TIMEOUT:-600}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/Volumes/LabWired/cargo-target/qemu-bridge-spike}"

for t in dtc fdtput fdtget mcopy; do
    command -v "$t" >/dev/null || { echo "missing $t (brew install dtc mtools)" >&2; exit 1; }
done
DEBUGFS="$(command -v debugfs || echo /opt/homebrew/opt/e2fsprogs/sbin/debugfs)"
[[ -x "$DEBUGFS" ]] || { echo "missing debugfs (brew install e2fsprogs)" >&2; exit 1; }

mkdir -p "$IMAGES" "$WORK" "$LOGS"

# --- 1. Raspberry Pi OS Lite arm64 (cached) --------------------------------
IMG_XZ="$(ls "$IMAGES"/*-raspios-*-arm64-lite.img.xz 2>/dev/null | tail -1 || true)"
if [[ -z "$IMG_XZ" ]]; then
    url="$(curl -sI https://downloads.raspberrypi.com/raspios_lite_arm64_latest \
        | awk 'tolower($1)=="location:"{print $2}' | tr -d '\r')"
    IMG_XZ="$IMAGES/$(basename "$url")"
    echo "downloading $url"
    curl -fSL -o "$IMG_XZ.part" "$url" && mv "$IMG_XZ.part" "$IMG_XZ"
fi
IMG="${IMG_XZ%.xz}"
[[ -f "$IMG" ]] || xz -dk "$IMG_XZ"
echo "image: $IMG"

# Partition table (MBR): p1 FAT boot, p2 ext4 root.
read -r P1_START P2_START P2_SECTORS < <(python3 -I - "$IMG" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read(512)
e = [struct.unpack("<II", d[446 + 16 * i + 8:446 + 16 * i + 16]) for i in range(2)]
print(e[0][0], e[1][0], e[1][1])
PY
)

# --- 2. kernel + DTB variants from the boot partition -----------------------
mcopy -o -i "$IMG@@$((P1_START * 512))" ::kernel8.img ::bcm2711-rpi-4-b.dtb "$WORK/"
# dtb_variant OUT STATUS_I2C1 STATUS_I2C3 (nodes found through /aliases)
dtb_variant() {
    cp "$WORK/bcm2711-rpi-4-b.dtb" "$1"
    fdtput -t s "$1" "$(fdtget "$1" /aliases i2c1)" status "$2"
    fdtput -t s "$1" "$(fdtget "$1" /aliases i2c3)" status "$3"
    # The PL011 (serial1 alias -> ttyAMA1) carries a Bluetooth serdev child;
    # drop it so the port is a plain tty usable as /dev/console.
    fdtput -r "$1" /soc/serial@7e201000/bluetooth 2>/dev/null || true
}
DTB_ONE="$WORK/bcm2711-rpi-4-b-lw.dtb"        # i2c1 on
DTB_TWO="$WORK/bcm2711-rpi-4-b-lw-dual.dtb"   # i2c1 + i2c3 on
DTB_OFF="$WORK/bcm2711-rpi-4-b-lw-off.dtb"    # i2c1 explicitly disabled
dtb_variant "$DTB_ONE" okay disabled
dtb_variant "$DTB_TWO" okay okay
dtb_variant "$DTB_OFF" disabled disabled

# --- 3. initramfs from the image's own busybox and i2c modules -------------
# No SD card: block I/O completes at host-timed points (and QEMU's SD model
# picks a random RCA), which breaks determinism under -icount.
ROOTFS="$WORK/rootfs.ext4"
if [[ ! -f "$ROOTFS" || "$ROOTFS" -ot "$IMG" ]]; then
    dd if="$IMG" of="$ROOTFS.part" bs=1048576 \
        skip=$((P2_START / 2048)) count=$((P2_SECTORS / 2048)) 2>/dev/null
    mv "$ROOTFS.part" "$ROOTFS"
fi
KVER="$("$DEBUGFS" -R "ls /usr/lib/modules" "$ROOTFS" 2>/dev/null \
    | tr -s ' ' '\n' | grep -E '^[0-9].*-v8$' | head -1)"
[[ -n "$KVER" ]] || { echo "no -v8 kernel modules in image" >&2; exit 1; }
IRD="$WORK/initramfs-src"
rm -rf "$IRD" && mkdir -p "$IRD"
dump() { "$DEBUGFS" -R "dump $1 $IRD/$2" "$ROOTFS" 2>/dev/null; [[ -s "$IRD/$2" ]] || { echo "dump $1 failed" >&2; exit 1; }; }
LIBDIR=/usr/lib/aarch64-linux-gnu
dump /usr/bin/busybox busybox
dump $LIBDIR/ld-linux-aarch64.so.1 ld.so
dump $LIBDIR/libc.so.6 libc.so.6
dump $LIBDIR/libresolv.so.2 libresolv.so.2
MODDIR=/usr/lib/modules/$KVER/kernel/drivers/i2c
dump $MODDIR/busses/i2c-bcm2835.ko.xz i2c-bcm2835.ko.xz
dump $MODDIR/i2c-dev.ko.xz i2c-dev.ko.xz
xz -d "$IRD"/*.ko.xz
INITRD="$WORK/initramfs.cpio"
python3 -I "$HERE/mkinitramfs.py" "$INITRD" \
    dir:bin dir:dev dir:lib dir:lib/aarch64-linux-gnu dir:lib/modules \
    dir:proc dir:sys dir:tmp chr:dev/console=5,1:600 \
    file:init="$HERE/guest/init":755 \
    file:bin/busybox="$IRD/busybox":755 link:bin/sh=busybox \
    file:lib/ld-linux-aarch64.so.1="$IRD/ld.so":755 \
    file:lib/aarch64-linux-gnu/libc.so.6="$IRD/libc.so.6":755 \
    file:lib/aarch64-linux-gnu/libresolv.so.2="$IRD/libresolv.so.2":644 \
    file:lib/modules/i2c-bcm2835.ko="$IRD/i2c-bcm2835.ko" \
    file:lib/modules/i2c-dev.ko="$IRD/i2c-dev.ko"
echo "initramfs: $INITRD ($KVER, sha256 $(shasum -a 256 "$INITRD" | cut -c1-16))"

# --- 4. QEMU and the LabWired bridge ----------------------------------------
[[ -x "$QEMU" ]] || "$HERE/build.sh"
"$QEMU" -device help | grep -q labwired-mmio-bridge || "$HERE/build.sh"
"$QEMU" -device labwired-mmio-bridge,help | grep -q irq-or || "$HERE/build.sh"
(cd "$REPO" && CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --release -p qemu-bridge)
BRIDGE="$CARGO_TARGET_DIR/release/qemu-bridge"

for d in "$DTB_ONE" "$DTB_TWO" "$DTB_OFF"; do
    "$BRIDGE" dt-report --dtb "$d" >"$LOGS/dt-report-$(basename "$d" .dtb).txt"
done
"$BRIDGE" dt-report --dtb "$DTB_ONE" --json >"$LOGS/dt-report-$(basename "$DTB_ONE" .dtb).json"

# The guest runs on one of the four raspi4b CPUs (maxcpus=1). With four,
# dual-bus runs under -icount were not byte-reproducible (1 in 4 diverged by
# ~100 ns of guest time at ~2.19 s): QEMU's round-robin TCG interleaves vCPUs
# at points that the host thread scheduling can move. One guest CPU still
# runs both bus loops concurrently (they sleep on the bus). GUEST_MAXCPUS=4
# (or empty) restores all four.
GUEST_MAXCPUS="${GUEST_MAXCPUS-1}"
APPEND="console=ttyAMA1,115200 earlycon=pl011,mmio32,0xfe201000 rdinit=/init${GUEST_MAXCPUS:+ maxcpus=$GUEST_MAXCPUS}"

# Deterministic guest clock: 2^ICOUNT_SHIFT ns of virtual time per guest
# instruction, no sleeping on idle (virtual time jumps to the next timer), no
# pacing against the host clock.
ICOUNT_SHIFT="${ICOUNT_SHIFT:-0}"
ICOUNT=(-icount "shift=$ICOUNT_SHIFT,sleep=off,align=off")
SOCKS="$WORK/sock"

# run NAME DTB CLOCK GUEST [bridge-options...]
#   CLOCK: icount|host   GUEST: single|dual (kernel arg lwtest=)
#   bridge-options: --attach SPEC, --no-irq-or, or --no-bridge
run() {
    local name="$1" dtb="$2" clock="$3" guest="$4"; shift 4
    local log="$LOGS/$name.log" sockdir="$SOCKS/$name" tdir="$LOGS/$name.transcripts"
    local bpid="" extra=() clk=() attach=() planflags=() nobridge=0
    while (( $# )); do
        case "$1" in
            --attach) attach+=(--attach "$2"); shift 2 ;;
            --no-irq-or) planflags+=(--no-irq-or); shift ;;
            --no-bridge) nobridge=1; shift ;;
            *) echo "run: bad option $1" >&2; exit 2 ;;
        esac
    done
    : >"$log"
    rm -rf "$sockdir" "$tdir"; mkdir -p "$sockdir" "$tdir"
    [[ "$clock" == icount ]] && clk=("${ICOUNT[@]}")
    echo "=== run $name (dtb=$(basename "$dtb"), clock=$clock ${clk[*]}, guest=$guest) ===" | tee -a "$log"
    local socks=()
    if (( ! nobridge )); then
        local line
        while read -r line; do
            [[ -z "$line" ]] && continue
            echo "LW-HOST: plan: $line" | tee -a "$log"
            extra+=(-device "${line#-device }")
            socks+=("$(sed -E 's/.*socket=([^,]*).*/\1/' <<<"$line")")
        done < <("$BRIDGE" plan --dtb "$dtb" --socket-dir "$sockdir" "${planflags[@]}")
    fi
    if (( ${#socks[@]} )); then
        "$BRIDGE" serve --dtb "$dtb" --socket-dir "$sockdir" --transcript-dir "$tdir" \
            "${attach[@]}" ${BRIDGE_EXTRA:-} 2>"$LOGS/$name.bridge.log" &
        bpid=$!
        local s
        for s in "${socks[@]}"; do
            for _ in $(seq 100); do [[ -S "$s" ]] && break; sleep 0.1; done
        done
    else
        echo "LW-HOST: no bridge devices for this run" | tee -a "$log"
    fi
    local t_start=$SECONDS
    "$QEMU" -M raspi4b "${clk[@]}" -kernel "$WORK/kernel8.img" -dtb "$dtb" \
        -initrd "$INITRD" -append "$APPEND lwtest=$guest lwcount=${DUAL_COUNT:-200}" \
        -display none -monitor none -serial stdio \
        -no-reboot "${extra[@]}" </dev/null >>"$log" 2>&1 &
    local qpid=$! t=0
    while kill -0 "$qpid" 2>/dev/null; do
        if grep -q '^LW-END' "$log"; then sleep 2; kill "$qpid" 2>/dev/null || true; break; fi
        if (( t >= BOOT_TIMEOUT * 2 )); then
            echo "LW-HOST: TIMEOUT after ${BOOT_TIMEOUT}s" | tee -a "$log"
            kill "$qpid" 2>/dev/null || true; break
        fi
        sleep 0.5; t=$((t + 1))
    done
    wait "$qpid" 2>/dev/null || true
    echo "LW-HOST: wall $((SECONDS - t_start))s to LW-END ($name, clock=$clock)" | tee -a "$log"
    if [[ -n "$bpid" ]]; then
        sleep 1
        kill "$bpid" 2>/dev/null || true
        wait "$bpid" 2>/dev/null || true
    fi
}

ONE_BME=(--attach i2c1:bme280@0x76)
TWO_BME=(--attach i2c1:bme280@0x76 --attach i2c3:bme280@0x77)
RUNS="${RUNS:-positive positive-2 no-device no-bridge host-1 host-2 dual dual-2 dual-noor disabled}"
for r in $RUNS; do
    case "$r" in
        positive|positive-2) run "$r" "$DTB_ONE" icount single "${ONE_BME[@]}" ;;
        no-device) run "$r" "$DTB_ONE" icount single ;;
        no-bridge) run "$r" "$DTB_ONE" icount single --no-bridge ;;
        host-1|host-2) run "$r" "$DTB_ONE" host single "${ONE_BME[@]}" ;;
        dual|dual-2) run "$r" "$DTB_TWO" icount dual "${TWO_BME[@]}" ;;
        dual-noor) run "$r" "$DTB_TWO" icount dual "${TWO_BME[@]}" --no-irq-or ;;
        disabled)
            # The loader must refuse to wire a part to a disabled node.
            "$BRIDGE" serve --dtb "$DTB_OFF" --socket-dir "$SOCKS" "${ONE_BME[@]}" \
                >"$LOGS/disabled.serve.txt" 2>&1 || true
            run "$r" "$DTB_OFF" icount single ;;
    esac
done

for r in $RUNS; do cat "$LOGS/$r.log"; done >"$LOGS/all-runs.log"

# --- 5. verdict from the guest consoles and bridge transcripts --------------
fail=0
pass() { echo "PASS $1"; }
failc() { echo "FAIL $1"; fail=1; }
# Guest console lines end in CR LF; match on the lines without the CR.
check() { if grep "$2" <(tr -d '\r' <"$LOGS/$1.log") >/dev/null; then pass "$1: $3"; else failc "$1: $3"; fi; }
has() { [[ " $RUNS " == *" $1 "* ]]; }
T1=i2c1.transcript

for r in positive positive-2; do
    has $r || continue
    check $r '^LW-DETECT-0x76: present' "i2cdetect shows 0x76"
    check $r '^LW-I2CGET: rc=0 out=0x60' "i2cget 0xd0 = 0x60"
    check $r '^LW-HOST: plan: -device labwired-mmio-bridge,addr=0xfe804000,size=0x1000,.*,irq=117$' \
        "bridge address/IRQ from the DTB plan"
done
for r in host-1 host-2; do
    has $r || continue
    check $r '^LW-I2CGET: rc=0 out=0x60' "i2cget 0xd0 = 0x60 (no -icount)"
done
for r in no-device no-bridge; do
    has $r || continue
    check $r '^LW-DETECT-0x76: absent' "i2cdetect shows no 0x76"
    check $r '^LW-END' "guest finished"
done
if has dual; then
    check dual '^LW-HOST: plan: .*addr=0xfe804000,.*irq=117$' "plan bridges i2c1 at 0xfe804000"
    check dual '^LW-HOST: plan: .*addr=0xfe205600,size=0x200,.*irq=117$' "plan bridges i2c3 at 0xfe205600"
    check dual '^LW-BUS1-ADDRS: 76 $' "i2cdetect bus 1 shows only 0x76"
    check dual '^LW-BUS3-ADDRS: 77 $' "i2cdetect bus 3 shows only 0x77"
    check dual '^LW-I2CGET bus1 0x76: rc=0 out=0x60' "i2cget bus 1 0x76 chip-id 0x60"
    check dual '^LW-I2CGET bus3 0x77: rc=0 out=0x60' "i2cget bus 3 0x77 chip-id 0x60"
    check dual '^LW-CONCURRENT bus1 0x76: ok=200 bad=0' "concurrent loop bus 1: 200/200"
    check dual '^LW-CONCURRENT bus3 0x77: ok=200 bad=0' "concurrent loop bus 3: 200/200"
fi
if has dual; then
    python3 -I "$HERE/irq-replay.py" "$LOGS/dual.transcripts" or | sed 's/^/  dual: /'
    if python3 -I "$HERE/irq-replay.py" "$LOGS/dual.transcripts" or | grep -qE '^overlap: [1-9]'; then
        pass "dual: transfers on bus 1 and bus 3 overlap in virtual time"
    else failc "dual: transfers on bus 1 and bus 3 overlap in virtual time"; fi
fi
# Negative control for the shared-IRQ OR. With irq-or=off each bridge writes
# the GIC input with its own level (last writer wins). Replaying the real
# transcripts with that rule shows the input at 0 while a bridge holds its
# level at 1. The guest still completes: Linux requests SPI 117 IRQF_SHARED,
# so every interrupt runs both BSC handlers, and each polls its own S
# register; a hidden level is picked up at the other bus's next interrupt.
if has dual-noor; then
    check dual-noor '^LW-HOST: plan: .*irq-or=off$' "plan with irq-or=off"
    replay="$(python3 -I "$HERE/irq-replay.py" "$LOGS/dual-noor.transcripts" last)"
    sed 's/^/  dual-noor: /' <<<"$replay"
    if grep -qE '^hidden \(last\): [1-9][0-9]* episodes' <<<"$replay"; then
        pass "dual-noor: without the OR the GIC input hides a pending bridge level ($(grep '^hidden' <<<"$replay"))"
    else
        failc "dual-noor: without the OR the GIC input hides a pending bridge level"
    fi
fi
if has disabled; then
    if grep -q '^/soc/i2c@7e804000 .* disabled ' "$LOGS/dt-report-$(basename "$DTB_OFF" .dtb).txt"; then
        pass "disabled: dt-report classes i2c1 as disabled"
    else failc "disabled: dt-report classes i2c1 as disabled"; fi
    check disabled '^LW-HOST: no bridge devices for this run' "plan makes no bridge"
    if grep -q 'i2c1 is disabled in the DTB' "$LOGS/disabled.serve.txt"; then
        pass "disabled: serve refuses --attach i2c1"
    else failc "disabled: serve refuses --attach i2c1"; fi
    check disabled '^LW-DETECT-0x76: absent' "guest shows no bus-1 device at 0x76"
fi

# Timing: START of the i2cget transfer -> DONE (IRQ) in the transcript must
# equal START + addr + reg + Sr + addr + data + STOP = 39 SCL periods, with
# SCL period = DIV / core clock (DIV as the guest wrote it, core clock as the
# guest's clk_summary reports it).
if has positive; then
    if python3 -I - "$LOGS/positive.transcripts/$T1" "$LOGS/positive.log" <<'PY'
import re, sys
tr = [l.split() for l in open(sys.argv[1])]
log = open(sys.argv[2], errors="replace").read()
clk = int(re.search(r"^LW-CLK: +vpu +\S+ +\S+ +\S+ +(\d+)", log, re.M).group(1))
div = [int(l[3], 16) for l in tr if l[1] == "WRITE" and l[2] == "0x14"][-1]
# i2cget write msg: A=0x76, DLEN=1, C = I2CEN|ST|INTT
i = max(k for k in range(2, len(tr))
        if tr[k][1:4] == ["WRITE", "0x00", "0x00008280"]
        and tr[k - 2][1:4] == ["WRITE", "0x0c", "0x00000076"]
        and tr[k - 1][1:4] == ["WRITE", "0x08", "0x00000001"])
t_start = int(tr[i][0])
j = next(k for k in range(i + 1, len(tr)) if tr[k][1] == "SYNC" and tr[k][4] == "1")
t_done = int(tr[j][0])
periods = 1 + 9 + 9 + 1 + 9 + 9 + 1
expect = -(-periods * div * 10**9 // clk)
print(f"  core clock (guest clk_summary vpu) = {clk} Hz, DIV written = {div}")
print(f"  SCL period = {div} / {clk} Hz = {div * 10**9 / clk:g} ns, bus = {clk / div:g} Hz")
print(f"  i2cget START write t={t_start}, DONE/IRQ SYNC t={t_done}")
print(f"  measured {t_done - t_start} ns, formula {periods} periods x "
      f"{div * 10**9 / clk:g} ns = {expect} ns")
sys.exit(0 if t_done - t_start == expect else 1)
PY
    then pass "timing: i2cget START->DONE equals the formula"
    else failc "timing: i2cget START->DONE equals the formula"; fi
fi

if has positive && has positive-2; then
    if cmp "$LOGS/positive.transcripts/$T1" "$LOGS/positive-2.transcripts/$T1"; then
        pass "determinism: icount transcripts byte-identical ($(wc -l <"$LOGS/positive.transcripts/$T1" | tr -d ' ') lines)"
    else failc "determinism: icount transcripts byte-identical"; fi
fi
if has dual && has dual-2; then
    for t in "$LOGS"/dual.transcripts/*.transcript; do
        b="$(basename "$t")"
        if cmp "$t" "$LOGS/dual-2.transcripts/$b"; then
            pass "determinism: dual-bus $b byte-identical ($(wc -l <"$t" | tr -d ' ') lines)"
        else failc "determinism: dual-bus $b byte-identical"; fi
    done
fi
if has host-1 && has host-2; then
    if cmp "$LOGS/host-1.transcripts/$T1" "$LOGS/host-2.transcripts/$T1"; then
        failc "negative control: transcripts without -icount differ"
    else pass "negative control: transcripts without -icount differ"; fi
fi

for f in "$LOGS"/dt-report-*.txt; do
    echo "  $(basename "$f" .txt): $(tail -1 "$f")"
done
for r in $RUNS; do
    echo "  $r: $(grep -h '^LW-HOST: wall' "$LOGS/$r.log" | cut -d' ' -f3-5) guest uptime at end" \
        "$(grep -h '^LW-UPTIME' "$LOGS/$r.log" | cut -d' ' -f2)s"
    [[ -f "$LOGS/$r.bridge.log" ]] && grep -h "QEMU disconnected after" "$LOGS/$r.bridge.log" | sed 's/^/      /'
done
echo "logs: $LOGS (all-runs.log = every guest console, <run>.transcripts/ = bridge)"
if (( fail )); then echo "RESULT: FAIL"; exit 1; fi
echo "RESULT: PASS"
