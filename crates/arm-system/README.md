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
The interrupt-begin hook bounds consecutive exception entries without actual
retirement: the ninth entry fails explicitly instead of allowing an unmapped
abort vector to loop indefinitely inside one native execution call. Retirement
resets the count, so recurring SVCs and valid handlers remain executable. Before
poisoning, the error records both the first and current PC, exception index,
IFAR/IFSR, DFAR/DFSR, SCTLR, TTBR0/1 and DACR. These diagnostics preserve the
original fault even when later abort-vector faults overwrite the live registers.

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

`execute_traced` executes one native instruction and returns its actual PC and
encoding only when an instruction retires. It reads through the physical pages
already resolved for the current translation block, including a Thumb-2
instruction crossing a native page boundary. It does not treat a virtual PC as
a physical address or invoke a speculative translation. Ordinary `execute`
does not perform these additional trace reads. Zero-retirement IRQ entry and
parked WFI return no instruction trace.

The pinned ARM decoder treats the ARM `0xeafffffe` and Thumb `0xe7fe`
unconditional branches to themselves as WFI. The native CMake wrapper preserves
their architectural branch behavior by generating a decoder copy in the build
directory (Cargo `OUT_DIR`). It requires exactly one occurrence of each
`if(insn == ...)` predicate, then disables only those two special cases; the
ordinary upstream branch decoder handles the instructions. A predicate-count
change fails the build. Vendored bytes remain unchanged, other instruction hints
are unaffected, and real WFI still parks the CPU.

The native wrapper builds a derived `arm-helper.c` in `OUT_DIR` to correct the
pinned Cortex-A7 capability declaration. Its A7 case enables NEON and VFP4 but
omits the VFP3 and FP16 decoder prerequisites, and reports the FPU-only D16
profile. The generated case adds `ARM_FEATURE_VFP3` and `ARM_FEATURE_VFP_FP16`,
retains VFP/VFP4/NEON, and sets MVFR0/MVFR1 to `0x10110222`/`0x11111111`.
The complete original A7 case must occur exactly once and its original helper
must be in the target source list; otherwise configuration fails. Only that
case changes. Vendor bytes and upstream headers/licenses are preserved.

This NEON MPE profile follows [ARM DDI 0462E, section 1.1 and tables 2-2, 2-3,
2-6 and 2-7](https://documentation-service.arm.com/static/5fa6919ab1a7c5445f2929ec?token=).
The [RV1103 datasheet](../../docs/assets/luckfox-pico-mini-a/rv1103-datasheet.pdf),
sections 1.1 and 1.2.1, identifies its Cortex-A7 as including NEON and FPU.
The pinned model's r0p0 MIDR/FPSID (`0x410fc070`/`0x41023070`) remain unchanged;
the reference manual's later r0p4 revision is not substituted for them.
Native instruction fixtures check privileged IDs with FPEXC.EN clear/set and
after reset, forbidden user ID reads, VFP3 immediates, D16/Q8 data paths, SIMD
integer/float addition and fused FMA, and scalar/SIMD half conversions. They
explicitly grant CP10/11 access with ASEDIS/D32DIS clear. These tests do not
establish complete Cortex-A7 conformance, general CPACR/ASEDIS/D32DIS enforcement,
deprecated VFP short-vector behavior, Linux userspace boot or physical acceptance.

The wrapper also generates an `exec.c` copy that clears the block-table pointer
after its real release in `code_gen_free`. Upstream leaves that pointer dangling;
if cache growth and its old-size fallback both fail, guarded execution returns
an error but subsequent disposal frees the table again. The generated copy
requires exactly one matching release and retains upstream license headers.
Subprocess regressions check both safe poisoned-CPU disposal after two failed
allocations and continued retirement after successful old-size fallback.

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
Before `tlib_init`, the bridge configures the JIT translation cache to a minimum
of 8 MiB and a maximum of 16 MiB. The pinned 1 MiB default is smaller than its
3 MiB maximum generated-block reserve plus 0.9 MiB search-data reserve. That
causes the high-water calculation to underflow and lets guest translations
overwrite the native JIT prologue. The bridge checks the pinned reserve constants
before initialization and fails explicitly if the minimum becomes insufficient.
Upstream cache growth and flushing operate within the configured bound; the
disposal wrapper also applies during growth and reuse. The diagnostic
`translation_cache_status` query reports the current size and allocation count,
including initial allocation, growth and reuse at the cap. It does not fetch
guest memory or provide restorable CPU state.
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
Additional regressions verify translated MMU trace encodings, cross-page Thumb-2
traces, repeated ARM/Thumb self-branch retirement with unchanged PC and no waiting,
and real WFI parking. An owned subprocess with a timeout verifies that an unmapped
abort vector returns a poisoning error with first/current fault diagnostics;
repeated retiring SVCs and the valid guest-abort test verify the bound's recovery
behavior. A cache-pressure subprocess retires 200,000 distinct-PC single
instructions, checks the final PC/register state, and requires growth from 8 to
16 MiB plus reuse at the cap. The same stream reproduces a native SIGSEGV with
the unsafe upstream cache default.
These tests establish architecture integration, not a complete Linux boot or
physical hardware acceptance.

Vendored source and license provenance: `../../third_party/tlib-PROVENANCE.md`.
Native builds use local vendored source and make no network requests. This
crate is workspace-only (`publish = false`).
