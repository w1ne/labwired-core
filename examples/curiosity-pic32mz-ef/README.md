# Curiosity PIC32MZ EF 2.0 (PIC32MZ2048EFM144) — first MIPS board

The Microchip **Curiosity PIC32MZ EF 2.0 Development Board** (DM320209)
carries a **PIC32MZ2048EFM144**: a PIC32MZ EF MCU (MIPS32 M5150 core with
FPU, 200 MHz, 2 MiB program flash, 512 KiB RAM). This is LabWired's first
**MIPS** part and the first board on its MIPS core.

What runs: the new MIPS core (`crates/core/src/cpu/mips.rs`: the MIPS32
release 2 integer ISA with branch delay slots, CP0, eight shadow register
sets, and the PIC32MZ interrupt controller (EVIC) folded into the core in
external-interrupt-controller mode), UART2 (`pic32_uart`, the PKoB4 virtual
COM port, interrupt driven in both directions) and Timer1 (`pic32_timer`).
**Unmodified upstream Apache NuttX** (apache/nuttx 7956039dfd, nuttx-apps
1f8cac8b87, 2026-10-08) built for an out-of-tree board
(`crates/firmware-curiosity-pic32mz-nuttx/board`) starts at the reset vector
0xBFC00000 and passes:

| Script | Firmware | What it proves |
| :--- | :--- | :--- |
| `nuttx-nsh.yaml` | NuttShell | reset vector → boot flash startup → shadow-set GP init → C runtime → nx_start → UART2 console; `hello` typed on the console runs the builtin app (task create/exit/waitpid) and returns to the prompt |
| `nuttx-ostest.yaml` (manual) | `testing/ostest` | 34 test groups, 0 errors: signals, POSIX timers, message queues, mutexes/semaphores/condvars, cancellation, round-robin preemption on the Timer1 tick, priority inheritance |

```sh
cargo run -q -p labwired-cli -- test --script examples/curiosity-pic32mz-ef/nuttx-nsh.yaml
examples/curiosity-pic32mz-ef/ci/test.sh
# ~9 minutes: ostest spins the CPU by design.
cargo run -q -p labwired-cli -- test --script examples/curiosity-pic32mz-ef/nuttx-ostest.yaml
```

Rebuild the fixtures (byte-identical):
`crates/firmware-curiosity-pic32mz-nuttx/build.sh nsh|ostest`.

Why a pinned upstream commit and not a release: before 7956039dfd ("record
the running task on interrupt exit"), NuttX's PIC32MZ interrupt path never
updated `g_running_tasks[]`, so the first context switch corrupted the Idle
task's saved registers. The model reproduced that crash on NuttX 12.11.0
before the fix was found upstream; no release tag (13.1.0 and earlier)
carries it yet.

Not modelled yet: ports and PPS (so no LEDs or switches), the FPU,
microMIPS, the DSP ASE, the TLB, caches, DMA, SPI, I2C and Timers 2-9. See
`VALIDATION.md`.
