# Prospective actual SPI writeback AHB fault

## Landed checkpoint — 2026-10-14

Runtime [PR28](https://github.com/CrispStrobe/codal-samd/pull/28) merged as
`374348d256299f3cd46fc33a3e7287863f12aa6c`, source
`e81fa3b95a3f3e0a32975273f7da0d176ffd61b8`, after both enabled checks passed.
Engine [PR233](https://github.com/CrispStrobe/labwired-core/pull/233) merged as
`1ec61766775897fa9eeb9f8edc4a0a796def4527`, source
`e5726080426b58b916f6c2f24294db767fefc3f7`, after all 23 enabled checks passed
and only seven declared optional jobs skipped. Reviewed/tested/landed trees,
exact source/base parents and default-branch refs were verified. Engine tree is
`bf0732fa4d9fb96558824b3e8f66c855884bb32e`; runtime tree is
`e654b0041caa5d94af89bd9c04a755c4cee4680f`. No consumer pin changed.

Final actual [run38609722076/job116009634346](https://github.com/CrispStrobe/labwired-core/actions/runs/38609722076/job/116009634346)
passed all four cases. Both fault profiles completed at 148174 steps, preserving
18 prefix and 28 recovery rows, nine actual fiber swaps, DATA bytes `91 82 73`,
wire byte `91` and frozen pre-fault remaining count 30. Full actual DMA/SPI IRQ
entries and returns were 3/2; the recovery-only 2/2 projection is explicitly
derived after verifying the complete census. One actual TERR reached exception47
with typed `dma_error`, PRIMASK0, released CS and strict disabled cleanup.
Callback return, actual Event/main resume and positive recovery were observed.
Both positive profiles completed at 126844 steps with 28 rows, six swaps and 2/2
actual DMA/SPI entries and returns. Driver/model and four-million-step bound
were unchanged; this is named controller-corruption evidence, not spontaneous
valid-buffer or physical failure, full native Arcade, installed app or RTx.

The official sole original is 170574 bytes, SHA256
`b9e2e06b5efc60b929a407b26dac28f2dc72ac10edf4c14331407a24f171f236`.
Before acquisition, the independent exact-source/image-bound reader was frozen
at SHA256 `17e4a54f9bebf9886415e25badcde15ed90637e6b0c125e9eb46af0a4109d82e`,
hash-checking unchanged predicate parent
`22852f9c102e4897531caf8da24f758b3a12fab9f815c4e584e895a91dd9f706`.
It passed nonvacuous fault/envelope/provider positive and refusal controls, then
all four actual cases without replay or producer-helper import. Final tested
merge `75a4ea34dc18ca8f9791b913c3b28328397495d4` matched the reviewed tree,
with exact base `194dd59b17efa6506e1a0e936621d5f7eeb2ac02` and source parents.
Earlier runtime dependency-census and engine generated-inventory failures remain
failures; the final inventory correction restored only a renderer-required EOF
blank line, not executable code or a weakened gate.

See the [complete runtime source/image/original record and preserved failures](https://github.com/CrispStrobe/codal-samd/blob/b170bec1acb7e59f8d48406c4126e687bd06c949/docs/REAL-SPI-DMA-ERROR-RESULTS.md).
Next are separately qualified stale software-pended IRQ dispatch and blocking
terminal-result ownership, before full board/image/application/performance lanes.

## Retained acceptance contract

The fixture separately selects caller mode 12 from immutable runtime
`e81fa3b95a3f3e0a32975273f7da0d176ffd61b8`. It adds no driver/model repair.
This is a controller-corruption/writeback fault, not an ordinary valid-buffer
request or physical fault. The real guest worker globally disables DMAC before
snapshotting remaining count and retargeting enable-protected WBADDR to aligned
unmapped `0x60000000`; masking interrupts alone does not freeze DMA ticks. It keeps
the original valid buffer and live channel descriptor, then resumes global DMA.

The raw prefix must include partial accepted/wire progress, the worker, global
disable, all four WBADDR bytes, global re-enable, one further accepted DATA beat,
actual TERR at IRQ47 entry, CS release, typed failure callback/return, worker
completion and Event/main resume. Preserve every byte and the original RAM
writeback snapshot. Do not treat that old RAM count as post-fault writeback:
the controller now targets the unmapped address. No host flag/IRQ/Event injection
or callback substitution is allowed.

Require exactly one `dma_error` callback in IRQ47/PRIMASK0 with CS released,
strict pre-recovery disabled MASTER `0x2000c`, INTEN0 and DMA TX `0x200d00`, busy
reentry and actual callback return/main resume. Then require the unchanged
28-row positive asynchronous/blocking recovery, two later DMA-completion IRQs
and two SPI-TXC IRQs, with matched total DMA/SPI entry/return counts `[3,2]`.
Only the error IRQ contributes TERR; it must not masquerade as TCMPL or TXC.

The workflow builds fault mode12 and positive mode8 from two separate clean
pinned core trees, runs both scheduler profiles and retains all four originals
even if a case fails. Synthetic deletion/corrupt-byte controls are mandatory;
an independent source/image-bound reader must be frozen before original
acquisition. Source/compiler success is not guest admission. Preserve the first
actual failure before repair, keep the four-million-step bound and all enabled
exact-head checks, and normally merge only after reviewed/tested/landed tree and
parent/ref verification. A shared driver fix needs fresh affected eight-case
success/cancellation/timeout coverage as well as the new error caller.

See the [source caller/acceptance contract](https://github.com/CrispStrobe/codal-samd/blob/e81fa3b95a3f3e0a32975273f7da0d176ffd61b8/docs/REAL-SPI-DMA-ERROR-QUALIFICATION.md)
and [landed timeout boundary](samd-real-spi-timeout-qualification.md).
Stale IRQ, blocking-result lease, lifetime, native Arcade, installed app, RTx,
physical behavior and distribution/licence qualification remain separate.
No consumer pin, threshold, acknowledgement or physical capture changes.
