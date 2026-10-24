# Actual SPI destruction-policy qualification contract

## Landed actual checkpoint — 2026-10-15

Runtime [PR34](https://github.com/CrispStrobe/codal-samd/pull/34) landed as
`580931ac258ffa4b1949165247418b4e2eeb4b99`; engine
[PR239](https://github.com/CrispStrobe/labwired-core/pull/239) landed as
`6cc18e36497423993e0224114f1bc759160d3631`. Both Runtime checks and all23 enabled
engine checks passed, with only seven declared optional jobs skipped. Reviewed,
actual-tested and landed trees and merge parents/default refs matched.
[Final actual run38688077422](https://github.com/CrispStrobe/labwired-core/actions/runs/38688077422)
and the frozen independent source/image/provider-bound reader pass all ten cases.
Read the [exact sources/images, original hashes, preserved ratchet failure,
results and concrete next board/API blockers](https://github.com/CrispStrobe/codal-samd/blob/2677b3795a3d5dd989bddb89c5285db06d2c13a0/docs/REAL-SPI-LIFETIME-RESULTS.md).
No driver/model, consumer pin, threshold, ACK or physical capture changed.
This is deliberate invalid-destruction fail-stop plus settled idle reconstruction,
not safe active cancellation-and-delete, native Arcade, RTx or image admission.

## Retained fixture and observation contract

This fixture selects immutable Runtime
`932d476344107386d56dfd2d595d74d713780563` and unchanged driver/model.
Modes15/16/17 require exact deliberate reason `0xc42` at the actual panic PC,
never a timeout, bus fault or arbitrary panic. They respectively retain an active
token, execute inside the real delivering callback, or retain a blocking terminal
token before the original owner resumes. Mode18 settles two real transfers,
virtually deletes through SPI, observes destructor/remove/free returns and
preserved mask, constructs a fresh instance and completes two further transfers.
Keep Timer, MessageBus and buffers alive; address reuse is not simultaneous identity.

Each actual runner prints `SAMD_REAL_SPI_LIFETIME_RESULT ` followed by JSON schema1:
mode/profile/source/image, full selected provenance, chip/system paths, exact
steps/budget, terminal step/context,32 post-terminal steps, startup/vector/data/BSS
proofs, full IRQ counts, swaps/restores, fault registers, symbol PCs and ordered
rows. Each row carries step/kind/value/pc/lr/fiber/mask/exception/r0_r3. Device and
MMIO observer rows retain pre-step CPU context; manually sampled API/IRQ contexts
are actual observation points. Context is evidence, not an invented callback.
For refusals, terminal.step/context bind the first actual target_panic entry
with r0 0xc42;32 additional real steps must produce and retain the expected
panic status. Idle success binds the first actual 0x600d boundary; its
panic_reason is unsigned0. Fault-register values are actual readbacks with
mandatory zero assertions, not synthesized status fields.

Baseline witnesses retain the existing fifteen-value ordering. Lifetime values:
stage,object,fresh,actor,mask,ipsr,returned,return_mask,return_ipsr,return_actor,
owner_identity,canceller_identity,cancel_result,owner_returned,failures,writeback,
remaining,expected_panic. Mode15's remaining count is before deletion; DMA/wire
may continue while interrupts are masked, including post-panic observation.
Preserve every byte rather than falsely freezing that count. Mode17 freezes its
snapshot only after actual cancellation disables the channel.

Roles: select/accept/wire/release; irq-enter/irq-return; callback/callback-return;
async-resume; blocking-call/blocking-return; destroy/delete/d0/dtor/free/remove/
ctor/cancel/owner/canceller each suffixed -call or -return; enroll-call/return,
notify-call/return, panic, cancel-mmio0..3, dtor-mmio-spi/dtor-mmio-dma. Byte rows
are actual bus observations. Return values are actual r0; entry arguments retain
r0-r3. Mode17 enrollment is owner source1023 and allocated value; NotifyOne is
the actual cancellation Event source1022/same value/mode1, not an unrelated Event.

Deleting destructor D0 and complete/base D1/D2 targets are ELF-derived, with
aliases deduplicated. A missing required target or call refuses qualification;
do not manufacture an inlined call. Listener removal means actual MessageBus
remove return, not immediate private-node disappearance. NVIC teardown is bounded
by actual enable-register reads at destructor entry/return, not fabricated SCS
write observations. Peripheral interrupt-clear stores remain raw attributed rows.

The hosted workflow separately builds modes15–18 and8, executes all ten cases
despite individual failures and retains original output. A frozen independent
source/image/provider reader and nonvacuous controls must precede sole original
acquisition. Every enabled final-head check and clean actual caller must pass
before matched-head merge/tree/parent/ref verification. This is not natural
stale IRQ/timeout, arbitrary destruction/concurrency, complete native Arcade,
installed consumer, RTx, physical behavior or distribution/licence admission.
