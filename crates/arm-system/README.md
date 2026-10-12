# Embedded ARM system backend

`labwired-arm-system` implements a privileged ARMv7 Cortex-A7 in the simulator
process using the pinned QEMU-derived tlib translator. The default feature set
is empty; construction returns an explicit feature-required error.

Native support is opt-in and currently supports x86_64 Linux GNU only:

```sh
cargo test -p labwired-arm-system --features native --test architecture
cargo build -p labwired-arm-system --features native --profile native-system
```

The native feature refuses panic=abort builds. `native-system` inherits release
optimization and uses panic=unwind, allowing the Rust bus callbacks to catch
panics and return before C performs its own unwind. Both callback failures and
native fatal errors (including locally bound native abort/assert paths) poison
the CPU. This is not containment of memory corruption or operating-system faults. `reset` recreates native state and restores
registered executable ranges. Guest architectural exceptions remain normal
execution results; an IRQ entry can retire zero instructions without waiting.

All instruction fetches, MMU page-table reads, RAM and MMIO use `PhysicalBus`.
The bridge never maps a host RAM pointer. Register IDs 0..15 are R0..PC; 16 is
CPSR. Native callback widths 1/2/4/8 and transaction state are passed unchanged.
This upstream Cortex-A7 currently supplies transaction state zero; the wrapper
does not invent privilege/security metadata. External CP15 accesses forward to
the bus; the bus must return an explicit error for unknown registers. Callback
arguments are canonical register keys, independent of instruction opcode,
condition, destination register and read/write direction. The 32-bit layout is
`(opc1 << 21) | (CRn << 16) | (opc2 << 5) | CRm`; the 64-bit layout is
`(opc1 << 4) | CRm`. The bridge normalizes both compact native keys and literal
fallback instructions to these layouts. A zero execution budget is rejected.
LPAE is explicitly refused and poisons the CPU. The native block-finished hook
checks TTBCR.EAE immediately after its CP15 write ends the translated block,
before a following instruction can access memory; execution also checks EAE
before entering the translator. Linux images must use short-descriptor
translation (`CONFIG_ARM_LPAE=n`).
Executable ranges are checked against the 32-bit physical space, then rounded
outward to native page boundaries (1 KiB in this pinned backend).

Each CPU owns a unique shared-library file inode in a private TempDir, loaded
with RTLD_LOCAL and built with -Bsymbolic. Loading one inode multiple times
would share native CPU globals and is deliberately avoided. Idle CPUs may move
between threads; a CPU is not Sync and execution requires exclusive access.
The callback context is scoped to one execution and cleared on every return.
The native link also wraps `free_code_gen_buf`: upstream unmaps both read/write
and executable pointers even when x86_64 uses one aliased mapping. A concurrent
CPU can reallocate that address between the two calls and lose its new JIT
buffer. The wrapper unmaps aliases once, distinct mappings separately, and
clears owned pointers before releasing memory. This fixes allocator ownership
without serializing execution or changing vendored source.
Each native CPU registers a private, initially zeroed exclusive-monitor state
with tlib. That storage belongs to its unique library; the bridge destroys the
native mutex and condition variable after native disposal, before unloading the
library frees the storage. Guest LDREX/STREX/CLREX therefore have an initialized
reservation domain even though only one Cortex-A7 is registered per library.

The bus must supply an allocation-unique, stable `code_identity`. Changing buses
flushes translated code and clears the native exclusive reservation, even when
the new bus reuses the same address or holds identical memory values. Regular
execution on one bus retains translated code. Host code edits require explicit
`invalidate_code`; guest stores use the native code-coherence mechanism.

Tests cover real ARM/Thumb-2 arithmetic, exact-width physical MMIO, VFP width8,
short-descriptor MMU translation, SVC, guest data abort, IRQ entry/deassertion,
WFI, external CP15 32/64, bus errors/panics/poison/reset, guest and host code
modification, bus replacement, cache retention, unique inodes, contained native
fatals, interleaved CPUs, forced concurrent MMIO callbacks, drop isolation and idle
thread moves, unaligned executable-page boundaries and zero/overflow validation.
A subprocess test covers real LDREX/STREX/CLREX success/failure, independent
exclusive monitors across CPUs, dropping another CPU, moving a reservation to a
new host thread, and clearing reservations on reset or physical bus replacement.
A deterministic subprocess preload probe reallocates a just-released JIT address
and verifies that disposal preserves its new owner. Eight threads repeatedly
create, execute and dispose native CPUs to exercise concurrent mapping ownership.
These tests establish architecture integration, not a complete Linux boot or
physical hardware acceptance.

Vendored source and license provenance: `../../third_party/tlib-PROVENANCE.md`.
Native builds use local vendored source and make no network requests. This
crate is workspace-only (`publish = false`).
