#!/usr/bin/env bash
# Boots the real Raspberry Pi OS arm64 kernel in QEMU raspi4b with I2C1 (BSC1)
# served by the LabWired BSC model over a unix socket, and checks from the
# guest console that a LabWired BME280 answers at 0x76.
#
# Runs (guest = kernel8.img + an initramfs built from the image's busybox and
# i2c modules; -icount unless noted):
#   positive     bridge + BME280          -> 0x76 present, i2cget 0xd0 = 0x60
#   positive-2   same again               -> transcript byte-identical (cmp)
#   no-device    bridge, no I2C target    -> 0x76 absent
#   no-bridge    QEMU's own BSC model     -> 0x76 absent
#   host-1/2     positive without -icount -> transcripts differ (proves the
#                determinism check can fail)
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

# BSC1 on BCM2711, from bcm2711-rpi-4-b.dtb:
#   /soc/i2c@7e804000 reg = <0x7e804000 0x1000>, /soc ranges map
#   0x7e000000 -> 0xfe000000, interrupts = <GIC_SPI 0x75 LEVEL_HIGH>.
BSC1_ADDR=0xfe804000
BSC1_SIZE=0x1000
BSC1_SPI=117

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

# --- 2. kernel + DTB from the boot partition --------------------------------
mcopy -o -i "$IMG@@$((P1_START * 512))" ::kernel8.img ::bcm2711-rpi-4-b.dtb "$WORK/"
DTB="$WORK/bcm2711-rpi-4-b-lw.dtb"
cp "$WORK/bcm2711-rpi-4-b.dtb" "$DTB"
I2C1="$(fdtget "$DTB" /aliases i2c1)"
echo "i2c1 node: $I2C1 reg=$(fdtget -t x "$DTB" "$I2C1" reg) interrupts=$(fdtget -t x "$DTB" "$I2C1" interrupts)"
fdtput -t s "$DTB" "$I2C1" status okay
# The PL011 (serial1 alias -> ttyAMA1) carries a Bluetooth serdev child;
# drop it so the port is a plain tty usable as /dev/console.
fdtput -r "$DTB" /soc/serial@7e201000/bluetooth 2>/dev/null || true

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
(cd "$REPO" && CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --release -p qemu-bridge)
BRIDGE="$CARGO_TARGET_DIR/release/qemu-bridge"

APPEND="console=ttyAMA1,115200 earlycon=pl011,mmio32,0xfe201000 rdinit=/init"
BOOT_ARGS=(-kernel "$WORK/kernel8.img" -dtb "$DTB" -initrd "$INITRD" -append "$APPEND")

# Deterministic guest clock: 2^ICOUNT_SHIFT ns of virtual time per guest
# instruction, no sleeping on idle (virtual time jumps to the next timer), no
# pacing against the host clock.
ICOUNT_SHIFT="${ICOUNT_SHIFT:-0}"
ICOUNT=(-icount "shift=$ICOUNT_SHIFT,sleep=off,align=off")

# run NAME MODE CLOCK   MODE: bridge|no-device|no-bridge  CLOCK: icount|host
run() {
    local name="$1" mode="$2" clock="$3" log="$LOGS/$1.log" sock="$WORK/$1.sock"
    local bpid="" extra=() clk=()
    : >"$log"
    [[ "$clock" == icount ]] && clk=("${ICOUNT[@]}")
    echo "=== run $name ($mode, clock=$clock ${clk[*]}) ===" | tee -a "$log"
    if [[ "$mode" != no-bridge ]]; then
        local flag=()
        [[ "$mode" == no-device ]] && flag=(--no-device)
        rm -f "$sock"
        "$BRIDGE" --socket "$sock" --transcript "$LOGS/$name.transcript" \
            "${flag[@]}" ${BRIDGE_EXTRA:-} 2>"$LOGS/$name.bridge.log" &
        bpid=$!
        for _ in $(seq 100); do [[ -S "$sock" ]] && break; sleep 0.1; done
        extra=(-device "labwired-mmio-bridge,addr=$BSC1_ADDR,size=$BSC1_SIZE,socket=$sock,irq=$BSC1_SPI")
    fi
    local t_start=$SECONDS
    "$QEMU" -M raspi4b "${clk[@]}" "${BOOT_ARGS[@]}" \
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
    echo "LW-HOST: wall ${SECONDS_DIFF:-$((SECONDS - t_start))}s to LW-END ($name, clock=$clock)" | tee -a "$log"
    if [[ -n "$bpid" ]]; then
        sleep 1
        kill "$bpid" 2>/dev/null || true
        wait "$bpid" 2>/dev/null || true
    fi
}

RUNS="${RUNS:-positive positive-2 no-device no-bridge host-1 host-2}"
for r in $RUNS; do
    case "$r" in
        positive|positive-2) run "$r" bridge icount ;;
        no-device) run "$r" no-device icount ;;
        no-bridge) run "$r" no-bridge icount ;;
        host-1|host-2) run "$r" bridge host ;;
    esac
done

for r in $RUNS; do cat "$LOGS/$r.log"; done >"$LOGS/all-runs.log"

# --- 5. verdict from the guest consoles and bridge transcripts --------------
fail=0
pass() { echo "PASS $1"; }
failc() { echo "FAIL $1"; fail=1; }
check() { if grep -q "$2" "$LOGS/$1.log"; then pass "$1: $3"; else failc "$1: $3"; fi; }
has() { [[ " $RUNS " == *" $1 "* ]]; }

for r in positive positive-2; do
    has $r || continue
    check $r '^LW-DETECT-0x76: present' "i2cdetect shows 0x76"
    check $r '^LW-I2CGET: rc=0 out=0x60' "i2cget 0xd0 = 0x60"
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

# Timing: START of the i2cget transfer -> DONE (IRQ) in the transcript must
# equal START + addr + reg + Sr + addr + data + STOP = 39 SCL periods, with
# SCL period = DIV / core clock (DIV as the guest wrote it, core clock as the
# guest's clk_summary reports it).
if has positive; then
    if python3 -I - "$LOGS/positive.transcript" "$LOGS/positive.log" <<'PY'
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
    if cmp "$LOGS/positive.transcript" "$LOGS/positive-2.transcript"; then
        pass "determinism: icount transcripts byte-identical ($(wc -l <"$LOGS/positive.transcript" | tr -d ' ') lines)"
    else failc "determinism: icount transcripts byte-identical"; fi
fi
if has host-1 && has host-2; then
    if cmp "$LOGS/host-1.transcript" "$LOGS/host-2.transcript"; then
        failc "negative control: transcripts without -icount differ"
    else pass "negative control: transcripts without -icount differ"; fi
fi

for r in $RUNS; do
    echo "  $r: $(grep -h '^LW-HOST: wall' "$LOGS/$r.log" | cut -d' ' -f3-5) guest uptime at end" \
        "$(grep -h '^LW-UPTIME' "$LOGS/$r.log" | cut -d' ' -f2)s"
    [[ -f "$LOGS/$r.bridge.log" ]] && grep -h "QEMU disconnected after" "$LOGS/$r.bridge.log" | sed 's/^/      /'
done
echo "logs: $LOGS (all-runs.log = every guest console, *.transcript = bridge)"
if (( fail )); then echo "RESULT: FAIL"; exit 1; fi
echo "RESULT: PASS"
