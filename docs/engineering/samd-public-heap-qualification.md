# Prospective real-heap public allocation qualification

This source-bound fixture consumes CODAL source
`1f95fec39e379205e1b4fc854faa26597b747452`. Its eight hosted builds record
43 selected source inputs each. Compilation passed; actual guest qualification
is still required. No allocator, whole-runtime, native Arcade, image, consumer
or RTx admission follows from compilation or manufactured predicate tests.

The fixture retains the ten service/clock cases and two explicitly raw-domain
alignment diagnostics. All modes except the existing long-gap mode retain
the four-million-step bound; long-gap retains twelve million. Both engine
profiles execute the same immutable source-built images.

Mode 6 requires exactly 641 guest allocation checks, real free/reuse, no
interrupts/context switches, and terminal success in masked thread context.
It covers simultaneous odd-sized allocations/churn, public realloc content
and arithmetic refusal, null/zero/calloc overflow, actual volatile 64-bit
objects/arrays, and raw-domain realloc/free. The raw realloc caller checks five
initialized bytes, not its entire rounded capacity or absence of every overread.

Mode 7 retains the configured fatal OOM behavior, expecting exactly
`0xe0000014` after actual heap exhaustion during realloc. The observer binds
the ELF heap extent, two live allocations' actual metadata/raw headers, the
guard's 131072-byte requested size, and the old 32-byte `0x69` payload after
refusal. No allocator-failure substitution or invented guest memory is used.
The old allocation remains live; this is not a returning NULL-on-OOM contract.

Unique BSS/vector/startup/source/image/map checks and independent architectural
IRQ/assembly observations remain mandatory. Positive checks do not admit
arbitrary invalid pointers, over-aligned allocation, general libc semantics,
zero-size C++ new or new-on-arithmetic-refusal paths, which remain untested.
Native SPI, selected board/CF2 APIs/notices, actual Arcade frames/input/debugger,
representative performance and installed-app adoption remain separate work.
