# Actual CODAL heap alignment diagnostic

This prospective diagnostic preserves the selected raw CODAL heap and the
existing ten service cases. It is not allocator admission or native Arcade
qualification. Execution belongs to the source-bound hosted workflow; local
manufactured witnesses are predicate controls only.

Runtime caller mode5 comes from `CrispStrobe/codal-samd` source
`92e99cc881db3e5d8f0186821d8c83bc241eb3b2`, with actual core
`2ed19d64ec3359143409a4b0a9758b4aeef3ee7d`. The workflow uses a sixth clean
core checkout. Existing modes0–3 keep their four-million-step bounds; mode4
keeps its separately named twelve-million-step bound. Mode5 is bounded by
four million actual machine steps in each scheduler profile.

The caller first completes its existing free/reuse heap control, then retains
two simultaneous eight-byte allocations before constructing Timer, bus or
fiber services. The expected old-source bug witness is exactly `0xe0000c60`,
stage1, checks1 and heap-reused1. Actual RAM addresses must be distinct,
four-byte aligned and twelve bytes apart; at least one must be four modulo
eight. They are observed guest values, never injected expected addresses.
Actual exception, SysTick entry/return and assembly-switch observations must
all remain zero, with PRIMASK1 and no pending SysTick at the terminal witness.

Each image retains unique aligned BSS witnesses, poisoned BSS/data startup
verification, vector verification and source/core/image/map provenance binding.
The nonignored manufactured controls reject wrong status, stages, sizes,
address domains/alignment/stride/overflow, service activity and architectural
state. They do not execute the guest or prove the bug happened.

Preserve the sole official original result before changing allocation semantics.
A passing expected-bug diagnostic is evidence of missing fundamental alignment,
not successful allocator behavior. A later adapter must preserve the actual raw
heap path, explicitly separate pointer domains and qualify allocation/free,
reallocation/content/failure and real eight-byte object accesses. Full image
licensing, native Arcade integration, performance and installed-app adoption
remain separate pending gates.
