# SK-AM62 (TI AM6234) — first AArch64 board

The TI **SK-AM62B-P1** starter kit carries an **AM6234**: an AM62x Sitara
SoC with four Arm **Cortex-A53** cores (Armv8.0-A, up to 1.4 GHz), a
Cortex-M4F and a device-management R5F. This is LabWired's first **AArch64**
part and the first board on its AArch64 core.

What runs: the new AArch64 core (`crates/core/src/cpu/aarch64.rs`: the A64
integer ISA, the SIMD&FP registers with the loads/stores and the few data
operations Zephyr emits, EL0-EL3 with the exception model, a stage-1 MMU, and
the GICv3 + generic timer folded into the core) on A53 core 0, with
MAIN_UART0 as the 16550 console. Stock, unmodified **Zephyr v4.3.0** images
for `sk_am62/am6234/a53` start at the ELF entry at EL3 and pass:

| Script | Firmware | What it proves |
| :--- | :--- | :--- |
| `zephyr-hello.yaml` | `samples/hello_world` | EL3 init → ERET to EL1 → MMU on → GICv3 + timer bring-up → MAIN_UART0 console |
| `zephyr-sync.yaml` | `samples/synchronization` | two threads alternating on a semaphore: virtual-timer PPI 27 through the GIC (IAR1/EOIR1), context switches, WFI idle |
| `zephyr-kernel-common.yaml` | `tests/kernel/common` | 12 ztest suites, 63/63: atomics (LDAXR/STLXR), bitarray (SIMD popcount, lazy FP trap), clock, irq_offload (SVC), user-mode cases at EL0 with per-thread page tables, printk, ... |

```sh
cargo run -q -p labwired-cli -- test --script examples/sk-am62/zephyr-hello.yaml
cargo run -q --release -p labwired-cli -- test --script examples/sk-am62/zephyr-kernel-common.yaml
examples/sk-am62/ci/test.sh
```

Rebuild the fixtures (byte-identical): `crates/firmware-sk-am62-zephyr/build.sh`.

Not modelled yet: A53 cores 1-3 (SMP), the M4F and R5F, UART interrupts (the
console is polled), GPIO/LEDs, the boot flow (TF-A, U-Boot), stage 2 and
16/64 KB granules. See `VALIDATION.md`.
