# Validation and acceptance boundaries

Preparation, loader qualification, guest boot, publication and physical acceptance are separate results.

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
