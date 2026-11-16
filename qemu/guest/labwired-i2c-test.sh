#!/bin/sh
# Runs as PID 1 (init=) inside the Raspberry Pi OS guest. Probes I2C bus 1,
# prints LW-* marker lines for the host, then reboots (QEMU -no-reboot exits).
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev 2>/dev/null
mount -t tmpfs tmpfs /tmp
echo "LW-BEGIN kernel $(uname -r)"
modprobe i2c-bcm2835 && echo "LW-MODPROBE i2c-bcm2835 ok"
modprobe i2c-dev && echo "LW-MODPROBE i2c-dev ok"
# give udev-less devtmpfs a moment to create the node
n=0; while [ ! -e /dev/i2c-1 ] && [ $n -lt 50 ]; do sleep 0.1; n=$((n+1)); done
echo "LW-I2C-NODES: $(ls /dev/i2c-* 2>/dev/null | tr '\n' ' ')"
for a in /sys/bus/i2c/devices/i2c-*; do
    [ -e "$a/name" ] && echo "LW-ADAPTER $(basename $a): $(cat $a/name)"
done
echo "LW-CMD: i2cdetect -y 1"
busybox i2cdetect -y 1 > /tmp/detect.txt 2>&1 || true
cat /tmp/detect.txt
if grep -q '^70:.* 76' /tmp/detect.txt; then
    echo "LW-DETECT-0x76: present"
else
    echo "LW-DETECT-0x76: absent"
fi
echo "LW-CMD: i2cget -y 1 0x76 0xd0"
v=$(busybox i2cget -y 1 0x76 0xd0 2>&1); rc=$?
echo "LW-I2CGET: rc=$rc out=$v"
dmesg | grep -i 'i2c' | sed 's/^/LW-DMESG: /'
echo "LW-END"
sync
echo b > /proc/sysrq-trigger
sleep 5
