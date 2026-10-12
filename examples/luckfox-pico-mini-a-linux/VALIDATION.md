# Validation and acceptance boundaries

Preparation, loader qualification, guest boot, publication and physical acceptance are separate results.

Production direct zImage acceptance **passed locally on 2026-10-12** through the shipped optimized native CLI. The complete [UART/boot receipt](evidence/production-boot-2026-10-12.json), [independent CLI identity](evidence/cli-identity-2026-10-12.json) and [gate input provenance](evidence/gate-inputs-2026-10-12.json) are retained. Input provenance is written before launch and deliberately says `guest_boot: NOT_RUN`; only the separate boot receipt records `ACCEPTED`.

| Accepted production run | Evidence |
|---|---|
| Source HEAD | `28bb66c5dbcc52ead1311664551d400db51026ec` |
| Build | `native-system`, release optimization with mandatory panic unwind; `qemu-arm-system` and default `event-scheduler` |
| Actual CLI SHA256 | `d2d11eef64c9879ab97adf164e7cae0b45fa08c5556ea19f55f1da7b490c0055` |
| Actual embedded DLL SHA256 | `fcef17db34d797231fbcb0d6f2b88b24a1fd4f3c0df37eff24505a0b11628ce0` |
| CPU / timer | CPU 24 MHz explicit functional override; default 1.2 GHz; architectural timer 24 MHz |
| Actual retired instructions | `730226752` |
| Cycles / Machine fuel | `754102272` each; includes `23875520` idle cycles |
| Wall elapsed / bound | `589.026260905` seconds / `1800` seconds |
| Kernel and userspace | `6.18.56-0-lts`, real PID1 `/init`, `armv7l`, mounted proc/sysfs/devtmpfs, cpuinfo and meminfo |
| Guest sleep / uptime | Actual guest `sleep 1`; `12.33` to `13.36`, delta `1.03` seconds |
| Last PC / stop | `0x801bef6c` / `userspace acceptance`; one UART acceptance marker |

The boot receipt rehashes zImage `eac9582162e912bdb577bba6ea224d98a29c997e52ee891383ea836aa704680a`, compiled DTB `5069562faf959fad32ce34db1dcc0f76f7721382abe6d0d713c033a6d0f83a75`, initramfs `b13962b76edb327b89cec464014cd2565694d88a7f0f86ccc361317cf879ccce` and kernel config `4e64812402b287e13fd7513aa830432782a1ae94a5461bfe579999d1e18432fb`. Official kernel/config/BusyBox and pinned `/init`/newc bytes were unchanged; no VFP, trace or kernel guard was bypassed.

The exact local gate used the debug test harness `target/debug/deps/rv1103_linux_boot-2afb81fe4604fcd0` to invoke the preserved **optimized production CLI**. The harness profile does not change the CLI being executed:

```sh
LABWIRED_RV1103_CLI=/tmp/labwired-rv1103-native-system-cli-20261012 \
LABWIRED_RV1103_ARTIFACTS=/tmp/labwired-rv1103-strict-artifacts-root-physical-20261012 \
LABWIRED_RV1103_BOOT_RECEIPT=/tmp/labwired-rv1103-production-optimized-zimage-receipt-20261012.json \
target/debug/deps/rv1103_linux_boot-2afb81fe4604fcd0 --ignored --exact production_zimage_reaches_pid1_and_advancing_guest_uptime --nocapture
```

That command executed **1 test: 1 passed, 0 failed**, in `595.33` harness seconds. Its original gate log is `/tmp/labwired-rv1103-production-optimized-zimage-gate-20261012.log`; rerunning requires a fresh receipt and provenance sidecar path. The gate validates CLI exit status, exact mode/board/kernel/PID1 identity, real UART proc/mount/uname evidence, unique finite uptime stamps and delta >=0.8, artifact hashes, compiled DLL SHA syntax and source lineage, explicit CPU/timer calibration, event/idle configuration, last PC, and exact limits. It checks `retirements + idle = cycles = fuel`, no partial failed window, wall <=1800 seconds, console <=64 KiB, retirement <=2 billion and cycles/fuel <=24 billion. It preserves failure console/last PC/stop/receipt instead of accepting a prior receipt, a raw Image diagnostic, a marker fixture or static RAM text.

The earlier debug production zImage run remains **FAIL**, at wall timeout `1800.04` seconds with `557223936` retired instructions and last PC `0x80c5fd70`; its receipt is `/tmp/labwired-rv1103-production-zimage-receipt-20261012.json`. A separate raw decompressed Image diagnostic reached userspace but cannot satisfy production zImage acceptance. The optimized production run above required no CPU/device/loader changes after that failed debug run. These results do not identify the debug timeout's cause or establish physical timing.

The deterministic pinned artifacts were prepared locally on 2026-10-12. Preparation independently rehashed official APK cache bytes; checked package identity, selected regular members, archive bounds, extracted hashes and kernel builtins; generated newc before final DTB compilation; and verified actual compiled DT geometry, references, bootargs and exact initrd end. The committed manifest contains these reproducible input identities. It says `guest_boot: NOT_RUN`; it is not a boot receipt.

Focused tests:

```sh
python3 -m unittest discover -s scripts/tests -p test_prepare_rv1103_linux.py
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=0 cargo +1.95.0 test -p labwired-core --test rv1103_linux_loader
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=0 LABWIRED_RV1103_ARTIFACTS=/tmp/rv1103-linux cargo +1.95.0 test -p labwired-core --test rv1103_linux_loader -- --ignored
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=0 LABWIRED_RV1103_ARTIFACTS=/tmp/rv1103-linux cargo +1.95.0 test -p labwired-core --features qemu-arm-system,event-scheduler --test rv1103_linux_loader -- --include-ignored
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=0 CARGO_PROFILE_DEV_PANIC=unwind LABWIRED_RV1103_ARTIFACTS=/tmp/rv1103-linux cargo +1.95.0 test -p labwired-cli --features qemu-arm-system --test rv1103_linux_cli -- --include-ignored
```

`dtc` and `fdtput` are required by the compiled-DTB fixture tests. The explicitly ignored pinned-artifact test requires a fresh prepared directory; without the feature it proves backend refusal preserves every nonzero RAM byte, and with the feature it checks native entry register readback and loaded RAM. Neither proves that Linux has booted. Native CLI integration also builds an executable, so its command explicitly overrides the development profile to panic unwind; the native containment guard remains mandatory. The bounded native CLI test executes 32 real zImage instructions and must return a failure receipt without userspace acceptance.

Python tests cover malicious/duplicate/missing archive members, member/cumulative/count limits, full gzip expansion including zero padding, hidden PAX/GNU physical record counts, individual/cumulative extension metadata before semantic TAR parsing, exact expansion boundaries, truncated physical payloads, download size/hash before cache commit, rehashing corrupt caches, fresh nonsymlink output, package/config mismatches, deterministic cpio, compiled geometry tampering and disabled timer rejection. Rust loader tests verify rejection reasons and preserve the entire nonzero RAM buffer for board, header, hash, DTB, config, overlapping and oversized input; genuine checked-overflow regressions previously panicked in `LinearMemory::load_from_segment` and now return false. A captured malformed-DTB regression checks zero, unaligned and truncated structure lengths, node names and property headers/data under `catch_unwind`; all must return an error and preserve RAM. Matching clock and GIC references cannot make reserved phandle `0xffffffff` valid; both preparation and loader tests reject it. Native positive loading compares every RAM byte against the exact three staged artifact ranges.

Guest PASS requires an actual production zImage run through the shipped loader and CLI. Raw decompressed Image diagnostics, static DTB/config checks, unit tests, string searches in RAM and a standalone userspace marker cannot satisfy this gate. The CLI reads only real UART2 output and requires complete kernel/PID1/mount/uname/proc evidence and a real guest sleep with uptime delta >=0.8 seconds. All limits and timing calibration appear in its create-new receipt.

An instruction tick is a functional approximation; the 24 MHz architectural counter is calibrated from the explicit CPU Hz. CPU deadline, native retirement and Machine quantum semantics remain authoritative. The runner does not skip Linux initialization or bypass guest syscalls to meet wall limits.

No physical hardware is available for this task. npm publication is intentionally out of scope. I2C/sensors, ESP/RF integration, external module build, full vendor BSP/SD boot and smartbasket application acceptance remain unfinished later gates.
