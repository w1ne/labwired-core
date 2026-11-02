# SK-AM62

The **TI SK-AM62B-P1** starter kit with an **AM6234**: an AM62x Sitara SoC
with four Arm Cortex-A53 cores (Armv8.0-A, 1.4 GHz), a Cortex-M4F and a
device-management R5F. It is LabWired's **first AArch64 part**.

LabWired runs A53 core 0 on its AArch64 core (the A64 instruction set, EL0-EL3,
a stage-1 MMU, and the GICv3 and generic timer inside the core), with
MAIN_UART0 as the console. **Unmodified upstream Zephyr v4.3.0** images for
`sk_am62/am6234/a53` start at the ELF entry at EL3:

| Script | Firmware | Result |
| :--- | :--- | :--- |
| `zephyr-hello.yaml` | `samples/hello_world` | banner + `Hello World! sk_am62/am6234/a53` on MAIN_UART0 |
| `zephyr-sync.yaml` | `samples/synchronization` | thread_a / thread_b alternate on virtual-timer interrupts |
| `zephyr-kernel-common.yaml` | `tests/kernel/common` | 12 suites, 63/63 (atomics, irq_offload, clock, user mode at EL0, …) |

!!! warning "Not claimed yet"
    Cores 1-3 (SMP), the M4F/R5F, UART interrupts (the console is polled),
    GPIO/LEDs, the boot flow, stage 2, 16/64 KB granules, Armv8.1+ and crypto
    instructions, and most SIMD/FP arithmetic. SIM-DERIVED: no bench part.

Evidence and the full claim table: `examples/sk-am62/VALIDATION.md`.
Rebuild the fixtures: `crates/firmware-sk-am62-zephyr/build.sh`.
