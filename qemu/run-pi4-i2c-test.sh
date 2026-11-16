#!/usr/bin/env bash
# Boots the real Raspberry Pi OS arm64 kernel in QEMU raspi4b with I2C1 (BSC1)
# served by the LabWired BSC model over a unix socket, and checks from the
# guest console that a LabWired BME280 answers at 0x76.
#
# Three runs:
#   positive   bridge + BME280        -> 0x76 present, i2cget 0xd0 = 0x60
#   no-device  bridge, no I2C target  -> 0x76 absent
#   no-bridge  QEMU's own BSC model   -> 0x76 absent
#
# Needs: qemu/build.sh done (or it runs it), cargo, dtc, mtools, e2fsprogs.
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
PARTUUID_DISK="$(python3 -I -c 'import sys;d=open(sys.argv[1],"rb").read(512);print("%08x"%int.from_bytes(d[440:444],"little"))' "$IMG")"

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

# --- 3. root fs: add the test script as init --------------------------------
ROOTFS="$WORK/rootfs.ext4"
dd if="$IMG" of="$ROOTFS" bs=512 skip="$P2_START" count="$P2_SECTORS" 2>/dev/null
"$DEBUGFS" -w "$ROOTFS" >/dev/null 2>&1 <<EOF
rm /labwired-i2c-test.sh
write $HERE/guest/labwired-i2c-test.sh /labwired-i2c-test.sh
sif /labwired-i2c-test.sh mode 0100755
sif /labwired-i2c-test.sh uid 0
sif /labwired-i2c-test.sh gid 0
EOF
"$DEBUGFS" -R "stat /labwired-i2c-test.sh" "$ROOTFS" 2>/dev/null | grep -q "Mode:  0755" \
    || { echo "failed to install guest script" >&2; exit 1; }

SD="$WORK/sd.img"
cp "$IMG" "$SD"
dd if="$ROOTFS" of="$SD" bs=512 seek="$P2_START" conv=notrunc 2>/dev/null
# QEMU's SD model needs a power-of-two card size.
size=$(stat -f %z "$SD" 2>/dev/null || stat -c %s "$SD")
p2=1; while (( p2 < size )); do p2=$((p2 * 2)); done
truncate -s "$p2" "$SD"

# --- 4. QEMU and the LabWired bridge ----------------------------------------
[[ -x "$QEMU" ]] || "$HERE/build.sh"
"$QEMU" -device help | grep -q labwired-mmio-bridge || "$HERE/build.sh"
(cd "$REPO" && CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --release -p qemu-bridge)
BRIDGE="$CARGO_TARGET_DIR/release/qemu-bridge"

APPEND="console=ttyAMA1,115200 earlycon=pl011,mmio32,0xfe201000 \
root=PARTUUID=$PARTUUID_DISK-02 rootfstype=ext4 rootwait ro \
init=/labwired-i2c-test.sh"

run() {
    local name="$1" mode="$2" log="$LOGS/$1.log" sock="$WORK/$1.sock" bpid="" extra=()
    : >"$log"
    echo "=== run $name ($mode) ===" | tee -a "$log"
    if [[ "$mode" != no-bridge ]]; then
        local flag=()
        [[ "$mode" == no-device ]] && flag=(--no-device)
        rm -f "$sock"
        "$BRIDGE" --socket "$sock" "${flag[@]}" 2>"$LOGS/$name.bridge.log" &
        bpid=$!
        for _ in $(seq 100); do [[ -S "$sock" ]] && break; sleep 0.1; done
        extra=(-device "labwired-mmio-bridge,addr=$BSC1_ADDR,size=$BSC1_SIZE,socket=$sock,irq=$BSC1_SPI")
    fi
    "$QEMU" -M raspi4b -kernel "$WORK/kernel8.img" -dtb "$DTB" \
        -drive "file=$SD,if=sd,format=raw,snapshot=on" \
        -append "$APPEND" -display none -monitor none -serial stdio \
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
    if [[ -n "$bpid" ]]; then
        sleep 1
        kill "$bpid" 2>/dev/null || true
        wait "$bpid" 2>/dev/null || true
    fi
}

run positive bridge
run no-device no-device
run no-bridge no-bridge

cat "$LOGS/positive.log" "$LOGS/no-device.log" "$LOGS/no-bridge.log" >"$LOGS/all-runs.log"

# --- 5. verdict from the guest console --------------------------------------
fail=0
check() { if grep -q "$2" "$LOGS/$1.log"; then echo "ok   $1: $3"; else echo "FAIL $1: $3"; fail=1; fi; }
check positive '^LW-DETECT-0x76: present' "i2cdetect shows 0x76"
check positive '^LW-I2CGET: rc=0 out=0x60' "i2cget 0xd0 = 0x60"
check no-device '^LW-DETECT-0x76: absent' "i2cdetect shows no 0x76"
check no-device '^LW-END' "guest finished"
check no-bridge '^LW-DETECT-0x76: absent' "i2cdetect shows no 0x76"
check no-bridge '^LW-END' "guest finished"
grep -h "QEMU disconnected after" "$LOGS"/*.bridge.log 2>/dev/null || true
echo "logs: $LOGS (all-runs.log = every guest console)"
if (( fail )); then echo "RESULT: FAIL"; exit 1; fi
echo "RESULT: PASS"
