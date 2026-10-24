# Actual CODAL cancellation and recovery checkpoint

This separate fixture selects caller mode 9 from runtime source
`846601f69005d2cbd1c82f33e350301882ac4f80`, which fixes recovery controller-reset
initialization without weakening the model guard or changing the caller.
The prior source `9bdd9177a3d3b2200bbfd5bcdb1a3650b0c439a7` compiled; all nine
earlier images remained identical. Final-source build and actual guest outcomes
were required before qualification and now passed for both affected callers.
This is not complete TX-service, native
Arcade, physical behavior, installed-app or performance admission.

The selected driver runs a 32-byte RAM transfer. A distinct real
fiber's competing start must refuse busy; its real cancel call must produce
exactly one typed failure, correct argument/identity and busy callback reentry,
without success delivery. The real failure callback emits an actual Event to
the enrolled main fiber. No host callback/Event injection is used.

The observer preserves the entire cancelled phase, not a filtered prefix:
CS, every partial DATA/wire byte, actual failure entry/return, cancellation
return and resumed main. It binds accepted-byte count to the real DMA writeback
remainder after cancellation, before recovery resets the descriptor. Released
CS must precede failure delivery; return/wake ordering must match. Blocked
attempts must not replace the caller's buffer, callback or terminal ownership.

Recovery then requires the same two four-byte asynchronous/blocking transfers
and unchanged 28-row DMA/wire/TXC/CS/callback/fiber ordering as the positive
caller. Final callback/failure witnesses, actual exception entries/returns,
assembly swaps, vector/startup/data/BSS and immutable source/image/map must bind.
The four-million-step bound is unchanged. Manufactured missing-row, byte and
premature-release controls are not actual guest evidence.

Freeze a separate independent reader with source/image/metadata bindings before
acquiring actual originals. Both scheduler profiles and all enabled exact-head
checks are required. Runtime [PR22](https://github.com/CrispStrobe/codal-samd/pull/22)
and this fixture are now landed after those finite gates passed. Preserve the prior
positive guest and all failures; no consumer pin, threshold, acknowledgement or
physical capture changes are part of this slice. Error, gated timeout, stale
IRQ and blocking-terminal lease cases remain separate subsequent requirements.

The first actual source-08ee cancellation guest delivered one typed failure,
resumed main and retained writeback count 30 (two accepted beats), but failed
recovery: DMAC reset was requested while global enable remained set. Its original
failure is preserved. The corrected driver explicitly globally disables the
exclusively owned controller before reset. The model's reset guard is unchanged.
Because shared driver code changed, the workflow rebuilds both mode 8 and mode 9
from the same immutable corrected source using separate clean core checkouts,
then requires both positive and cancellation cases in both scheduler profiles.
Step errors now preserve observed raw rows before failing; no source budget,
ordering predicate or failure is weakened or relabelled as success.

## Final-source evidence and adoption — 2026-10-14

- Runtime source above landed in [CODAL PR22](https://github.com/CrispStrobe/codal-samd/pull/22)
  as `bdd4bede956634ca271c2d9670da98a9622a1d27`. Runtime source build and
  documentation-policy checks passed; reviewed/build-tested/landed tree is
  `44fa38159f5bb0c06ac04ea4395dbf0ae4f08498`.
- Engine source `00525a5ad3a0ece03018004489c3bf5f063bb28e` landed in
  [PR229](https://github.com/CrispStrobe/labwired-core/pull/229) as
  `49d7a2af8a7bf462c70424e1376beb100780287b` after all **23 enabled checks
  passed** and seven declared jobs skipped. Exact parents are base
  `b4f1f01175bcb0f902a6a1adfa86f82f7d260aaf` and tested source; reviewed,
  actual tested merge `34cefece18ae486f72ec0dd85fdcec690a588a7a` and landed
  trees equal `64a00a307db14a042725f651f210886251f6bf92`.
- [Actual guest run38537952558/job115765353858](https://github.com/CrispStrobe/labwired-core/actions/runs/38537952558/job/115765353858)
  passed both modes in both profiles. Mode 8: 126790 steps, 28 causal rows,
  six actual context swaps. Mode 9: 145294 steps, nine cancelled-prefix rows
  followed by the unchanged 28 recovery rows, eight actual swaps. Both retained
  matching DMA47/SPI79 exception entries and returns.
- Each cancelled transfer accepted two DATA bytes, emitted zero modeled wire
  bytes and retained writeback remainder 30. One actual typed failure, callback
  identity/context/return, busy refusal, cancellation return, Event/main resume
  and both subsequent successful transfers passed the independent audit.
- Corrected mode-8 ELF SHA256 is
  `9db4a75a31effe631325ef152d251d3a92488f6575bfdcf9b767123a9a80a290`;
  mode-9 is `377c4be4b57a5b36d32784e074724b391a3cbb058599e27a886303f2b738955b`.
  The sole official original log is 216579 bytes, SHA256
  `950533e7740a960ba793cf40fffef804e1a432974c4d12fef8e2d089a4610027`.
- Independent reader SHA256 is
  `f3fb068dcf75c9ca1cc7a2bbfb74f6b4fcf70888c381800b22a3955ea80ba5d5`,
  frozen before acquisition with two synthetic positives, 110 refusal controls
  and four combined-envelope refusals. No producer imports or guest replay
  were used for original-log inspection.

See the [runtime result record](https://github.com/CrispStrobe/codal-samd/blob/master/docs/REAL-SPI-SERVICES-RESULTS.md)
for separate build evidence, historical images and preserved failures. Next
qualify actual DMA error, clock-gated timeout, stale IRQ and blocking-result
ownership as separately selected callers. Clock-gated reads of zero are not
cleanup proof: restore the actual clock before checking disabled hardware and
preserve a failure before recovery changes that state. Then complete board/
CF2/API/build/notices/image admission, frames/input/debugger, representative
RTx and installed consumer qualification. No consumer pin, physical capture,
acknowledgement or performance threshold changed in this checkpoint.
