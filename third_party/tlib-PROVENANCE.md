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
