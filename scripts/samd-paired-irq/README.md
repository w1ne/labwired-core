# Prospective strict paired-IRQ guest harness

Guest qualification pending: the dedicated hosted workflow now pins CODAL
`3c66f0d27c865192a2b22217e71c1f88f2b3fe91` after all eight enabled source checks
passed on that exact head. These are source controls/builds, not guest results.
The separate receipt-control workflow runs manufactured Python tests and shell
checks only. Neither workflow's presence establishes runtime admission.
Historical IRQ fixtures remain unchanged.

The dedicated workflow selects that immutable source explicitly. `build.sh`
requires `LABWIRED_PAIRED_QUALIFIED_SOURCE`, a clean
source checkout, and builds five controls. This explicit parameter checks
identity, not GitHub qualification; the workflow/reviewer must verify the latter.

Execute the ignored `samd_paired_irq_guest` test twice, with ordinary scheduling
and `--features event-scheduler`, providing `LABWIRED_PAIRED_FIXTURE_DIR`.
Bind source files, compiler, linked ELF hashes, final qualification head/tree,
official run identity, and both execution logs in a reviewed receipt. Add
manufactured positive/refusal controls for that receipt reader before publishing
a success claim. `test_record.py` contains manufactured reader controls, not
actual guest results or an independent audit. The builder records the six
authored source files and cross-compiler driver hash before/after compilation;
the recorder checks them again, the actual qualification merge/head/parents,
five linked ELF hashes and both logs. The driver hash alone is not full
toolchain-component closure. No timeout, compiler failure, missing result, or unexpected
exception may count as a successful refusal.

The prospective test validates all vector words, startup BSS clearing and .data
copy, positive IRQ0 delivery only after the outer pair restores the mask, and
four exact faults: underflow3, changed diagnostic identity5, depth limit4, and
lost mask6. Each refusal must continue for 64 observed instructions, masked in
thread mode, with unchanged diagnostics and zero IRQ witnesses, inside the
actual compiled terminal function. Fault storage precedes an out-of-line
`InterruptMask::restore(1)` at the selected `-O0` build; only that exact compiled
helper is allowed during transition, for at most 32 instructions. The terminal
64-step count resets across the helper; unrelated code or a caller return
refuses. Four pure Rust progress controls cover transition, escape, stuck helper
and stopped PC. This is a bounded liveness observation, not
proof of indefinite behavior. The diagnostic identity is not a real fiber.

Still owed: hosted compile/format/lint checks, actual source-bound guest results,
full recording-entry-point and guest adversaries, independent receipt review,
real CODAL caller and scheduler integration,
peripheral IRQ/DMA, notices and consumer adoption. No ASF-free runtime, native
Arcade boot, physical safety or performance result is established by these
authored controls.

## Source gate — 2026-10-11

[CODAL PR10](https://github.com/CrispStrobe/codal-samd/pull/10) remains unmerged
pending affected guest qualification. The exact pinned source passed these eight
enabled checks; earlier failures remain in their original runs:

| Checks | Original successful run |
| --- | --- |
| Census controls | [38150739080](https://github.com/CrispStrobe/codal-samd/actions/runs/38150739080) |
| IRQ build | [38150739060](https://github.com/CrispStrobe/codal-samd/actions/runs/38150739060) |
| Foundation profile | [38150739096](https://github.com/CrispStrobe/codal-samd/actions/runs/38150739096) |
| Mask build | [38150739093](https://github.com/CrispStrobe/codal-samd/actions/runs/38150739093) |
| Transfer state controls and build | [38150739068](https://github.com/CrispStrobe/codal-samd/actions/runs/38150739068) |
| Paired policy/mutation controls and five ARM builds | [38150739139](https://github.com/CrispStrobe/codal-samd/actions/runs/38150739139) |

The pin qualifies only those source checks. Require successful actual guest
execution, source-bound original receipts and independent review before
representing these authored controls as guest-qualified or merging their source.
