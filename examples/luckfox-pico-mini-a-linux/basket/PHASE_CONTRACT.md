This is a distinct prepared Linux profile. Guest execution is **NOT_RUN**;
Task4C must prove the phases on the shared canonical circuit. Base `/init`,
its ten-node DT and base loader keep their original meaning.

`phases.json` is the reviewed shared authority for the host and generated
`phases.h`. The guest does not implement sensor registers. The host applies
ordinary `Machine.set_input_on("accelerometer", "x"|"y"|"z", value_g)`.
`configs/devices/adxl345.yaml` encodes DATA_FORMAT0x08 as256LSB/g, signed16
little-endian. Stimulus A(1,-0.5,0.25g) therefore yields(256,-128,64);
B(-0.25,0.5,-1g) yields(-64,128,-256). FIFO remains in its normal bypass
reset mode. The radio is `wifi`, an explicit LabWired behavioral ESP-AT mock;
there is no ESP firmware or RF acceptance.

The exact unique phase sequence is `stimulus_a`, `stimulus_b`, `external_off`,
`external_restored`, with sequences1..4. Each begins with
`LW_BASKET_PHASE_READY seq=N id=ID\n`, then a real100ms Linux poll wait before
any transfer. At functional24MHz a4096-cycle host advance window is about
171us, so the host must inspect the completed window and apply the phase
intervention before advancing another window. Markers do not stop the CPU;
Task4C must verify that its maximum window fits this wait. Every successful
phase ends with `LW_BASKET_PHASE_DONE seq=N id=ID\n`. Repeated, omitted or
out-of-order markers are failures, even if later success text is present.

For each powered phase the helper uses actual I2C_RDWR: POWER_CTL0x2d=8
and DATA_FORMAT0x31=8 single-message writes must return1; combined DEVID0xe5
and six axis-byte reads must return2. A and B require distinct values above;
restoration requires fresh B values. Each actual ioctl result is printed as
`LW_BASKET_IO kind=i2c_rdwr ret=R count=N errno=E`. DEVID and decoded axes are
printed in `LW_BASKET_DEVID` and `LW_BASKET_AXES` records. Negative syscall
returns carry positive errno; short returned message counts fail.

Each powered phase performs all seven `uart_exchanges` in metadata, using
raw8N1 B115200 on real `/dev/ttyS4`: AT, GMR, disconnected status, mock
connect, connected status, mock disconnect, disconnected status. Replies must
match exact CRLF bytes including the explicit LabWired mock identity. Writes
handle partial results, EINTR and EAGAIN. Poll and reads use one bounded500ms
monotonic deadline per command, continue through that deadline to detect extra
bytes, and reject timeout/short/oversized/non-ASCII/malformed replies. Logs
show command, actual written/received counts, errno and escaped reply bytes.
Before each command the real tty input is flushed with TCFLSH/TCIFLUSH;
Task4C must also prove the controller/peer generation invalidated stale FIFO
state when the rail changed, using controller/component traces.

In `external_off`, the host remains powered while its declared carrier
external3v3 disconnect gates both real peers. Combined DEVID read must
actually return-ENXIO(-6) and AT must receive zero bytes by500ms. Then the host
restores the same gate, supplies B and requires fresh successful I2C/UART
traffic in `external_restored`. The helper never fabricates a host plant fault,
ROM reboot or sensor state. Later Task4C owns the independent actual plant
fault/full host cut, zero-retirement hold and recovery in a fresh strict fixture.

The helper finishes with `LW_BASKET_HELPER_ACCEPTED`, returns0 and `/init`
records that actual exit status before `LW_BASKET_GUEST_ACCEPTED`. These texts
alone are insufficient: the future host gate checks ordered markers, returned
counts/bytes/errno, shared plant/controller/component traces and identities.
Any init/helper failure emits `LW_BASKET_FAILURE` and the real PID1 visibly
parks without printing guest acceptance. `/init` retains actual mount/PID1,
uname/proc CPU/memory/uptime and1-second sleep checks, loads both modules
through ordinary Busybox insmod, prints actual module exit codes and kernel
taint, and requires real resolved sysfs parents for ff460000/i2c-3 and
ff4e0000/ttyS4. Linux6.18.56 makes the tty class `device` symlink target the
serial-port parent, with exact physical/controller/port ancestry:
`/sys/devices/platform/ff4e0000.serial/ff4e0000.serial:C/ff4e0000.serial:C.P`.
The validator requires canonical nonnegative decimal C/P IDs bounded by the
kernel's signed decimal name format, matching controller IDs and exactly this
ancestry; it does not infer those IDs from ttyS4. The physical platform
device's `of_node` is independently resolved and must equal
`/sys/firmware/devicetree/base/serial@ff4e0000`. This follows the pinned
`serial_core.c` tty parent, `serial_base_bus.c` naming/parent construction and
`drivers/base/core.c` class link, and is source analysis rather than a live
guest sysfs observation. Focused tests run the actual init validator and reject
wrong controllers/DT nodes, extra ancestry, malformed and oversized paths.
Its small fixed transcript plus kernel console must stay below
the64KiB host capture limit in Task4C; no live transcript is claimed here.

ABI provenance is the original pinned linux-lts-dev APK, scanned through the
existing strict archive reader on every preparation. `pins.json` records the
selected original UAPI byte identities. ARM kernel termios is36bytes with
NCCS19 and no libc speed fields; pollfd8bytes; clock_gettime64(403) uses a
signed64 sec/nsec16-byte timespec; i2c_msg12bytes(buf offset8), i2c_rdwr8bytes.
TCGETS0x5401/TCSETS0x5402/TCFLSH0x540b come from those headers. The helper
uses poll168 for waits; legacy nanosleep162 would require the different
8-byte timespec32 and is deliberately unused. Static assertions cover all
structures actually crossing this boundary. No newlib/libc/stdio is linked.

Preparation requires the recorded default arm-none-eabi-gcc13.2.1 version.
It rechecks the reviewed original module receipt, original RK3x source,
untouched matching config/Module.symvers, every module import CRC, stock
signature and exact module bytes. It builds the static ELF in two distinct
work directories with epoch0/path maps and validates ET_EXEC/ELF32LE ARM
EABI5/no interpreter/dynamic/imports and bounded load ranges. Each directory
writes one sorted exact-whitelist newc and compiles the exact twelve-node DT;
actual helper/DT/archive bytes must agree between the two builds and with the
checked-in reviewed expected pins. The manifest reports these observations;
a user-edited manifest cannot authorize arbitrary content.

Run preparation with fresh output, verified offline APK/source cache and
original base/module artifacts:

```sh
python3 scripts/prepare-rv1103-basket.py \
  --output /tmp/rv1103-basket-prepared \
  --cache /tmp/labwired-rv1103-verified-apk-cache-20261012 \
  --base-artifacts /tmp/labwired-rv1103-strict-artifacts-root-physical-20261012 \
  --module-artifacts /tmp/labwired-rv1103-strict-modules-task1-version-fix-20261012
```

The output is published atomically only after guards pass. It contains no
concatenated overlay, duplicate archive path or extracted ARM host helper.
DT nodes add only I2C3ff460000/0x1000 SPI21 with RV1106/rk3399v1 fallback and
UART4ff4e0000/0x100 SPI29/alias serial4, on the existing fixed24M references.
Alias i2c3 binds `/dev/i2c-3`; aliases and clocks are checked in actual DTB.
The actual newc byte length determines initrd-end within64MiB and separate
kernel/DT/decompression ranges. Hardware is unavailable; preparation is not
physical acceptance or a live Linux basket proof.
