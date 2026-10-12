# Vendored tlib

Upstream: https://github.com/antmicro/tlib
Pinned commit: `167decf9129758829762e582939128c0694e90d6`.
Source archive: `https://codeload.github.com/antmicro/tlib/tar.gz/167decf9129758829762e582939128c0694e90d6`.
Archive SHA-256: `8db4d1fe0a316eec5dcae83bd4d2757b5172332493d6fdecfd9d4b81cb7a77a3`.

`third_party/tlib` is an unchanged extraction of that archive. The native bridge
and build wrapper live in `crates/arm-system/native`, outside the vendor tree.
Builds do not fetch sources, submodules, or dependencies from the network.
ARM32 uses the vendored softfloat-2 sources; softfloat-3 is a RISC-V dependency
and is not used or fetched by this configuration.

The upstream `LICENSE` states LGPL licensing unless a file says otherwise,
and contains LGPL 2.1 text. Per-file copyright and licensing headers are
preserved, including LGPL ARM/QEMU notices and permissive components.
The newly authored Rust/C bridge uses the workspace MIT license. The workspace
MIT declaration does not relicense upstream sources.
Distribution of a binary with this backend must preserve these notices and
provide the corresponding source and applicable license obligations.

The backend embeds an in-process QEMU-derived Cortex-A7 translator and privileged
ARMv7 architecture implementation. It launches no separate VM process. Supported
native build target/host: x86_64-unknown-linux-gnu. Explicit CMake settings:
HOST_ARCH=i386, HOST_WORD_SIZE=64, TARGET_ARCH=arm, TARGET_WORD_SIZE=32,
Release, and -Wl,-Bsymbolic. Native callbacks require panic=unwind; production
builds use the workspace `native-system` profile. The default feature set is
empty and does not invoke CMake.

The external bridge applies GNU linker `--wrap=free_code_gen_buf`. The pinned
upstream x86_64 allocator uses one mapping for both `tcg_rw_buffer` and
`tcg_rx_buffer`, but its disposal function unconditionally calls `munmap` on
both aliases. Between those calls another CPU can allocate the released address;
the second call can unmap that CPU's fresh JIT buffer. The bridge replaces this
internal disposal call with one unmap for aliased pointers and separate unmaps
only for distinct mappings, clearing owned pointers before release. Upstream
source bytes remain unchanged. A subprocess preload probe deterministically
reallocates the released address and checks that the replacement survives;
concurrent CPU construction/execution/disposal and repeated parallel architecture
runs test the host allocator ownership boundary.

The external CMake wrapper also builds a derived ARM decoder in its build
directory (Cargo `OUT_DIR`), leaving `third_party/tlib/arch/arm/translate.c`
byte-for-byte unchanged. The pinned decoder recognizes ARM `0xeafffffe` and
Thumb `0xe7fe` branches to themselves and substitutes WFI. The wrapper requires
exactly one `if(insn == 0xeafffffe)` and one `if(insn == 0xe7fe)` predicate,
replacing each with `if(0 /* LabWired: branch-self retires; it is not WFI */)`.
Only these special cases are disabled; the existing ordinary branch decoder
preserves retirement and the unchanged PC. Real WFI and other hint decoding
remain upstream behavior. A source-list or predicate-count mismatch fails the
build. The generated decoder retains upstream copyright/license headers and
remains derived from the LGPL source; it is not covered solely by the bridge's
MIT license. ARM/Thumb branch-self and real-WFI regressions verify this adaptation.

Before upstream initialization, the external bridge calls
`tlib_set_translation_cache_configuration` with an 8 MiB minimum and 16 MiB
maximum. The pinned 1 MiB default cannot contain `TCG_MAX_CODE_SIZE` (3 MiB)
plus `TCG_MAX_SEARCH_SIZE` (0.9 MiB); the resulting high-water underflow allows
translations to overwrite the JIT prologue. A compile-time check against the
maximum generated-block size and a guarded initialization check against the
full code/search reserve reject future incompatible constants. Upstream cache
allocation, growth and flushing are retained within the configured bound.
The size-change hook supplies the diagnostic `translation_cache_status` query
(current bytes and allocation count). An owned, timeout-bounded cache-pressure
subprocess checks 200,000 distinct-PC retirements, PC/register correctness,
8-to-16 MiB growth and capped-cache reuse; the unconfigured default reproduces
a native SIGSEGV. This corrects the allocation configuration rather than claiming
general containment of native memory corruption or operating-system faults.

The external CMake wrapper additionally generates an `exec.c` copy in `OUT_DIR`.
It requires exactly one `tlib_free(tbs)` call and adds `tbs = NULL` immediately
after the real release in `code_gen_free`. Without that ownership correction,
failure of both cache growth and its old-size fallback returns a guarded error,
but subsequent disposal frees the dangling table again. The generated source
retains upstream copyright/license headers and LGPL lineage; vendored bytes
remain unchanged. Timeout-bounded subprocess regressions exercise both failed
allocations with safe poisoned-CPU disposal, and successful old-size fallback
with correct subsequent retirement. This is a specific cleanup correction,
not a claim of general out-of-memory containment.

The external bridge enables the upstream interrupt-begin hook to bound repeated
exception entry without retirement. The ninth consecutive entry returns a native
fatal error and poisons the CPU instead of hanging within one execution call;
actual retirement resets the count. The error captures first-entry and current
PC/exception, IFAR/IFSR, DFAR/DFSR, SCTLR, TTBR0/1 and DACR before poisoning hides
the native register interface. An unmapped-abort-vector subprocess regression
checks the bound, diagnostics, poison and reset; valid abort handling and repeated
retiring SVCs remain covered. The hook and unwind stay in the authored C bridge;
fatal unwinding occurs after Rust physical-bus callbacks return.

The external CMake wrapper also generates `arm-helper.c` in `OUT_DIR`. It matches
exactly one complete original Cortex-A7 case, adds VFP3/FP16 decoder feature
prerequisites to the existing NEON/VFP4 declaration, and replaces that case's
FPU-only MVFR0/MVFR1 with the NEON MPE values `0x10110222`/`0x11111111`.
It preserves the r0p0 MIDR/FPSID and all other CPU cases, requiring the original
helper in the target source list. Vendored bytes and upstream license/copyright
headers remain unchanged; the generated helper retains the same LGPL lineage.
Primary evidence is [ARM DDI 0462E](https://documentation-service.arm.com/static/5fa6919ab1a7c5445f2929ec?token=),
section 1.1 and tables 2-2/2-3/2-6/2-7, and the [RV1103 datasheet](../docs/assets/luckfox-pico-mini-a/rv1103-datasheet.pdf),
sections 1.1/1.2.1. See the native crate README for tested capabilities and limits.
