# Prospective software-pended idle SPI IRQ qualification

## Landed checkpoint — 2026-10-14

Runtime [PR30](https://github.com/CrispStrobe/codal-samd/pull/30) merged as
`b991f75ea2be58e116b9f5a582a3d5fe168b10cc`, source
`35386bee113d4dc1531aa3c8ab90e6f9078191fe`, after both enabled checks passed.
Engine [PR235](https://github.com/CrispStrobe/labwired-core/pull/235) merged as
`d64ab20c6d92bc2b821e634aa0c4ffa355ff964c`, source
`28b93fea8307ded245baedffa6f318134997df26`, after all 23 enabled checks passed
and only seven declared optional jobs skipped. Reviewed/tested/landed trees,
exact source/base parents and default-branch refs were verified. Runtime tree is
`5d6059ee8ad9c067fe5883776779b01580fcf054`; engine tree is
`295c8864a478ccdac500a4149134ccdb5ffb18fe`. No consumer pin changed.

Actual [38628064916/job116072238678](https://github.com/CrispStrobe/labwired-core/actions/runs/38628064916/job/116072238678)
passed all four cases and the frozen source/image-bound original reader. Both
stale profiles completed at 156963 steps with 30 raw prefix and 28 recovery rows
and eight actual context swaps. The full actual DMA/SPI entries and returns
were 4/4: one prime hardware, one software-pended, two recovery completions each.
Software-only 1/1 is separately observed; recovery-only 2/2 is explicitly derived
after checking the complete prefix and census, never substituted for full counts.

The original retains all eight guest ISPR store bytes, actual exceptions47/79
entry and return, callback/Event/main and helper PC/LR settlement. During the
software epoch, DMA flags remained0 and SPI sticky flags3 with INTEN0/control
`0x2000e`/CS released; no extra callback, failure, DATA/wire or CS event occurred.
The positive controls remained 126844 steps, 28 rows, six swaps and actual 2/2.
Driver/model and the four-million-step bound were unchanged. Guest steps are
not RTx, and software-pended idle dispatch is not natural stale hardware IRQs,
stale timeouts, physical behavior or complete native Arcade admission.

The sole original is 227794 bytes, SHA256
`435fc53544e2c13f3534093148f8b7365c37c1d8ebc6f3fdce0d8a207d9cb9de`.
Before acquisition, official provider/source/tree/parents were admitted and the
reader was frozen at SHA256
`f4ee6e6b94611243f13673b2d2ea92e9212905dee22a74fc4b76a5952ab19b4f`,
hash-checking immutable predicate
`5e42460b160593bba20421f7cd968b9dbbca777a3a8972226043db76a5c0e82b`.
It passed case/envelope/provider refusal controls before all four originals;
no guest replay or producer-helper import was used. Tested merge
`7a893d01de7ee68d0b5a157c6e6d2dc33bbaef1f` had exact base
`e6b3a79c187d2d424f7253ca2d673fe638b90839` and source parents with the same tree.

See the [full source/image/original record and limits](https://github.com/CrispStrobe/codal-samd/blob/f689395d37177ea660600c0544803db52fc0ea4a/docs/REAL-SPI-STALE-RESULTS.md).
Next qualify blocking terminal-result ownership and lifetime before full
board/API/image/application/performance lanes. No acknowledgement, physical
capture, threshold, installed-app, distribution/licence or store claim changes.

## Retained acceptance contract

This is a separate mode13 caller at immutable runtime
`35386bee113d4dc1531aa3c8ab90e6f9078191fe`, with unchanged driver/model.
It is not a naturally arising stale hardware interrupt or stale timeout test.
The [landed DMA-error checkpoint](samd-real-spi-error-qualification.md)
and its original failures remain preserved.

A real prime transfer must finish through hardware DMA and SPI TXC, callback
entry/return, actual Event delivery and main-fiber resume. Preserve that entire
prefix before the real guest masks interrupts and stores bit31 to both NVIC
ISPR0/ISPR1, pending IRQ31/IRQ63. Observe every raw store byte, actual exception47
and79 entry/return, idle DMA flags, retained sticky SPI TXC with INTEN0, released
CS and unchanged control registers. No extra callback, failure, DATA/wire byte
or CS transition may occur while the pending IRQs settle. Observe the dispatch
helper's actual PC/LR return before admitting the unchanged28-row async/blocking
recovery. Never replace full observed counts with recovery-only counts.

The expected complete DMA/SPI entry and return census is4/4: one prime hardware
completion, one explicitly software-pended idle dispatch, then two recovery
hardware completions each. Each software entry and return is separately bound
to the actual guest stores and helper epoch. A counter or guest flag alone is
insufficient. Complete startup/vectors/data/BSS/heap/provenance, real context
swaps and the unchanged four-million-step budget remain mandatory.

Build mode13 and positive mode8 with separate clean pinned core checkouts,
run both profiles and retain all four originals even on failure. Nonvacuous
deletion, bad-byte, extra-effect and ordering controls must refuse. Freeze an
independent exact-source/image-bound reader before acquiring the sole original;
do not import producer helpers or replay guests to read evidence. Every enabled
final-head check must pass before matched-head merge and reviewed/tested/landed
tree/parent/ref verification. A shared driver/model repair needs fresh affected
prior caller qualification rather than a silent adoption.

Blocking terminal-result ownership, lifetime, full board/API/image/native
Arcade, installed application, RTx and physical behavior remain separate. No
consumer pin, threshold, acknowledgement or hardware capture changes.
