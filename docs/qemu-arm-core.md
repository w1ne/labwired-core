# Embedded QEMU-derived Cortex-A7 core

The Luckfox Pico Mini A uses an RV1103 with a Cortex-A7. The optional native adapter integrates its CPU
through the existing `Cpu` trait, `Machine`, `Bus`, and observer interfaces.
The embedding boundary is Unicorn 2.1.5: it already extracts QEMU CPU/TCG
code as a library and provides a Cortex-A7 model. Do not reimplement ARMv7
instruction decoding or launch a separate QEMU machine to bypass LabWired.

## First integration

- Native, optional `qemu-arm` feature; default and browser builds stay independent.
- One synchronous instruction per `Cpu::step`; default batching/scheduling.
- LabWired owns authoritative memory and devices. A native page cache supplies
  instruction bytes; guest loads/stores go through width-preserving Bus calls
  (1, 2 and 4 bytes; larger accesses fail explicitly).
- Register access and the standard instruction trace work through `dyn Cpu`.
- Bus failures stop execution, including failures inside native callbacks.
- CPSR/visible-register snapshots are diagnostic only; restore, IRQ delivery,
  MMU-enabled execution, Thumb IT blocks, WFE/WFI, and architectural exceptions explicitly fail until
  modeled. Never identify Cortex-A state as Cortex-M state.
- Instruction steps are not silicon cycle counts.

Unicorn is GPL-2.0; enabling this feature links that dependency. It is not
linked by default builds. Distribution of an enabled
binary must account for its upstream license.

## Validation contract

1. Add behavioral tests for arithmetic, ARM/Thumb stepping, register access,
   exact-width device accesses, bus error propagation, code invalidation,
   moving an idle core between threads, and unsupported capability rejection.
2. Implement the native adapter with scoped callbacks: borrowed buses must never
   remain registered after a step, even when setup or execution fails.
3. Extend the existing CPU trace conformance gate to exercise the optional core.
4. Run focused native tests, default conformance gates, formatting, and linting.

## Run the native CPU fixture

From the core repository root, with Rust and native CMake/Clang build tools:

```sh
cargo test --locked -p labwired-core --features qemu-arm --test qemu_arm_core --test cpu_trace_conformance
cargo test --locked -p labwired-cli --features qemu-arm --test cortex_a_cli
cargo run --locked -p labwired-cli --features qemu-arm -- run --firmware tests/fixtures/cortex-a7-core-smoke.elf --system examples/cortex-a7-core/system.yaml --max-steps 10
cargo run --locked -p labwired-cli --features qemu-arm -- test --script examples/cortex-a7-core/smoke.yaml --output-dir /tmp/labwired-cortex-a7-smoke
```

The checked-in [ARM assembly](../examples/cortex-a7-core/firmware.S) computes
7 + 5, writes **12** to `0x20000000`, then loops at ELF PC `0x1010`.
The [script](../examples/cortex-a7-core/smoke.yaml) checks that RAM result.
This is a synthetic initialized-memory platform with no RV1103 peripherals.
It must not be flashed onto a Luckfox board.

Rebuild the fixture with `arm-none-eabi-gcc`:

```sh
arm-none-eabi-gcc -mcpu=cortex-a7 -marm -nostdlib -Wl,--build-id=none,-Ttext=0x1000,-e,_start examples/cortex-a7-core/firmware.S -o tests/fixtures/cortex-a7-core-smoke.elf
```

Fixture compiler: GCC 13.2.1 (20231009). SHA-256:
`a7f3d90415d88d3c37e91a60b20580263ef4d6e687c001d1ff222cb4b14b8236`.
Default builds refuse Cortex-A7 with a `qemu-arm` requirement; browser builds
refuse it explicitly. The chip-only and legacy interactive CLI drivers direct
users to the system-aware route instead of executing Cortex-M.

## Executed evidence

On 2026-10-11/12, the native suite with `--features qemu-arm,event-scheduler`
passed **15 native CPU tests**, **3 per-core trace tests**, **1 Mini A declaration
test**, **11 snapshot honesty tests**, and **7 observer-store tests**.
The earlier feature-only (`qemu-arm`) run passed the initial 14 CPU tests and
3 trace tests. The default build's
`cargo test -p labwired-core --lib one_arch_policy` passed **8 tests**.
The native CLI suite passed **7 tests**, including RAM results, feature gating,
legacy-driver refusal, unsupported boot/snapshot refusal and mismatched ELF
architecture rejection. The checked-in `smoke.yaml` also ran through the real
CLI: **pass**, stop `max_steps`, **10 steps**, RAM assertion `0x20000000 = 12`.
The same CLI suite passed **7 tests with default features**, confirming the
feature-disabled refusal paths. That build used `CARGO_PROFILE_DEV_OPT_LEVEL=0`
and an isolated target directory; the native run used the workspace profile.
Host CLI/DAP/WASM compilation and native library Clippy (`-D warnings`) passed.
A separate `wasm32-unknown-unknown` compile with the core's `qemu-arm` feature
also passed: the native dependency remains excluded and execution unavailable.
Existing unrelated WASM warnings were emitted in Wi-Fi and GDB stub code.

These establish the tested CPU/bus contract, not Linux boot or Mini A support.
The [native CI lane](../.github/workflows/core-qemu-arm.yml) runs CPU/trace,
declaration, snapshot/observer, and
CLI tests in both feature configurations; local evidence does not claim CI ran.

## Linux and Luckfox follow-on

The separate optional `qemu-arm-system` backend now implements privileged
Cortex-A7/MMU and physical SystemBus access, GIC400, architectural timer and DW
UART2. On 2026-10-12 its optimized panic-unwind CLI passed a real reduced RV1103
direct zImage boot to PID1, mounted proc/sys/devtmpfs and completed guest sleep
with advancing uptime. See the [exact acceptance receipt and commands](../examples/luckfox-pico-mini-a-linux/VALIDATION.md).
This does not extend the older `qemu-arm` ELF adapter's MMU/sleep contract above.
CRU/reset, pinmux, GPIO, I2C, SD/eMMC, external modules and vendor BSP remain
later work. A reduced native Linux boot does not prove that a stock Luckfox
image boots, and the browser cannot execute this native backend.

The smart basket carrier connects ESP32-C3 UART Wi-Fi, an ADXL345 I2C sensor,
and a protected single-cell battery with charger/regulators. Its schematic,
PCB, component model coverage, power validation, and hardware acceptance are
separate artifacts from this CPU integration.

## Sources

- [Unicorn's QEMU CPU reuse](https://www.unicorn-engine.org/docs/beyond_qemu.html)
- [Upstream MMU and exception behavior](https://github.com/unicorn-engine/unicorn/blob/master/docs/FAQ.md)
- [Rust binding](https://docs.rs/unicorn-engine/2.1.5/unicorn_engine/)
- [Luckfox Pico Mini A](https://www.luckfox.com/Luckfox-Pico-Mini-A?ci=536)
