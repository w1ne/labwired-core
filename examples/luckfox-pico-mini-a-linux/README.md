# Luckfox Pico Mini A: reduced RV1103 Linux direct boot

This fixture runs an official pinned Alpine ARMv7 Linux zImage and static BusyBox through LabWired's privileged Cortex-A7, SystemBus, GIC400, architectural timer and DW UART2. It qualifies a reduced virtual Linux boot. Physical board, smartbasket application, I2C, ESP and RF acceptance remain separate gates.

Prepare without committing downloaded binaries:

```sh
python3 scripts/prepare-rv1103-linux.py --cache /tmp/rv1103-apk-cache --output /tmp/rv1103-linux
```

The output must be fresh. A hash-addressed cache is rehashed on every use; `--offline` forbids downloads. Preparation checks official package identities, exact members, kernel builtins, deterministic initramfs and the compiled DTB before issuing a `VERIFIED` preparation receipt. This does not claim guest acceptance.

Build and run the explicit optional native mode (x86_64 Linux GNU, panic unwind):

```sh
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo +1.95.0 build --locked -p labwired-cli --profile native-system --features qemu-arm-system,event-scheduler
./target/native-system/labwired rv1103-linux --artifacts /tmp/rv1103-linux --receipt /tmp/rv1103-boot.json --cpu-hz 24000000 --wall-seconds 1800
```

CPU Hz defaults to 1.2 GHz; boot at that default has not been verified here. One retired instruction advances one functional CPU tick; this is not physical timing. The accepted run and reproduction command explicitly use `--cpu-hz 24000000`, a functional calibration override recorded in the receipt. The architectural timer remains 24 MHz in both cases. Idle fast-forward and the existing event scheduler are enabled explicitly; the runner uses small bounded `Machine::advance` windows.

The [2026-10-12 production receipt](evidence/production-boot-2026-10-12.json) passed with the explicit 24 MHz CPU calibration. To reproduce that bounded acceptance gate, use a fresh receipt path and the actual shipped optimized CLI:

```sh
LABWIRED_RV1103_ARTIFACTS=/tmp/rv1103-linux \
LABWIRED_RV1103_CLI="$PWD/target/native-system/labwired" \
LABWIRED_RV1103_BOOT_RECEIPT=/tmp/rv1103-boot-new.json \
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
cargo +1.95.0 test --locked -p labwired-core --profile native-system --features qemu-arm-system,event-scheduler --test rv1103_linux_boot -- --ignored --nocapture
```

This explicitly ignored, feature-required test fails on missing prerequisites. It invokes the CLI once, independently rehashes all inputs and the executable, and checks the complete fresh receipt. The [native workflow](../../.github/workflows/core-qemu-arm.yml) offers the same production gate only through the manual `linux_acceptance` boolean; its structural/default-off suites run separately. Local acceptance does not claim that CI has run.

A PASS requires the actual UART console's Linux identity, PID1 `/init`, mounted proc/sys/devtmpfs, ARMv7 uname, cpuinfo and meminfo, and a real guest sleep whose `/proc/uptime` delta is at least 0.8 seconds. The userspace acceptance marker alone is insufficient. Failure receipts contain the full bounded console, exact loaded-artifact hashes, compiled native DLL hash, separate upstream source lineage, last PC, limits and stop reason. Receipts are create-new and never overwrite an earlier result.

DDR is 64 MiB at physical zero. zImage loads at `0x02000000`, DTB at `0x03000000`, and initramfs at `0x03100000`. The loader reserves `0x00008000..0x016e5850` for decompression/BSS and checks every range before modifying RAM. Entry is ARM/SVC, IRQ/FIQ masked, MMU/cache off, `r0=0`, `r1=0xffffffff`, `r2=DTB`.

The root DT compatibility is only `rockchip,rv1103`. Linux's genuine Generic DT machine fallback handles this reduced platform without claiming another SoC. CRU/reset, pinctrl, DMA, FIQ debugger, SD and unsupported devices are omitted. EVTSTREAM is explicitly disabled because the timer model refuses EVNTEN. Components can attach later to the same SystemBus; they have no acceptance claim here.

See [required references](REQUIRED_DOCS.md), [external components](EXTERNAL_COMPONENTS.md) and [validation boundaries](VALIDATION.md).
