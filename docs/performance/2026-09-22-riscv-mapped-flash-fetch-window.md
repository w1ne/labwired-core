# RISC-V mapped-flash fetch window

The declarative ESP32-C3 configuration places executable IROM in
`SystemBus::flash` at `0x4200_0000`. The RISC-V interpreter's 256-byte fetch
window previously admitted Flash-XIP peripherals and `extra_mem` (IRAM/ROM),
but not that ordinary mapped-flash region. Consequently every application
instruction fell through to `bus.read_u32` and repeated the full memory/MMIO
routing path.

Mapped, non-zero-base flash now fills the same fetch window when no peripheral
overlaps the address. Guest stores retain the existing overlap invalidation.
Synthetic base-zero flash remains uncached because debugger and unit-test code
may patch it directly between steps; overlapping peripherals retain routing
precedence.

On the repository ESP32-C3 perf fixture, the Callgrind slope changes from
274.4 to 201.8 host instructions per simulated instruction (-26.5%). A native
160,000,000-step run on the same VPS changes from 8.84 s to 4.23 s (2.09x
throughput), with the same 511.97-instruction average batch width.

The focused RISC-V test group covers decode-cache host mutation,
self-modifying IRAM, interrupts, timer/WFI behavior, machine boundaries, and a
new proof that non-zero mapped flash actually populates the vetted window.
