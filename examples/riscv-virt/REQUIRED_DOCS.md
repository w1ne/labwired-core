# Required Documents — RISC-V `virt` machine

Authoritative sources used by this example (retrieved 2026-10-23):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| Pinned upstream RISC-V `virt` platform | the platform definition | RiscV32 CPU (`rv32imac`, privileged 1.10, CLINT time provider, hart 0), DRAM at 0x80000000, NS16550 UART0 at 0x10000000 (PLIC source 3), PLIC at 0x0C000000 (52 sources, CPU context 11), CLINT at 0x02000000 (10 MHz, software/timer on CPU 3/7) |
| QEMU `virt` machine documentation | the platform convention | The same memory map (RAM 0x80000000, UART0 0x10000000, CLINT 0x02000000, PLIC 0x0C000000) and the RAM-size machine parameter; this descriptor models an 8 MiB window |
| PC16550D datasheet | the UART | THR/RBR @ +0x00, LSR @ +0x05, THRE bit 5, reset 0x60 |
| RISC-V privileged specification | the CPU | Machine-mode CSRs and the `time` CSR the CLINT timebase feeds |

Open items: the PLIC and CLINT MMIO windows are declared stubs (the engine has
no platform-level interrupt-controller model yet); the machine's RAM size is
modelled at 8 MiB of the parameterised window.
