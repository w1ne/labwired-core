# Opt-in SAMD51 manual storage: qualified source boundary

Backported from `CrispStrobe/labwired-core` PR249/PR252. The source and CI
receipts below remain bound to that fork; this tree carries the model, the
instruction-boundary commit, the opt-in factory and the hosted fixture test.
The hosted builder, mutation harness and exact external checkout pins remain in
the fork and are not part of this chunk.

This lane follows the [qualified public source census](https://github.com/CrispStrobe/codal-samd/pull/71).
It adds an explicitly selected manual NVMCTRL slice, CMCC controls and a hosted
caller of the real public ZFlash implementation. The selected guest qualification
is recorded below. [PR249](https://github.com/CrispStrobe/labwired-core/pull/249)
merged as `7da375fc8ae4f61c191502a542a2d92e6ab9f948` after all 29 enabled
exact-head checks passed, with seven source-declared conditional skips. The
landed tree and ordered parents match the qualified composition.
Default chip descriptors and consumer pins are unchanged.

The new factory names are `samd51_nvmctrl_manual_storage` at `0x41004000` and
`samd51_cmcc_control` at `0x41006000`; both refuse IRQ/config overrides. The
profile requires exactly 512 KiB of zero-based flash, 512-byte pages and 8 KiB
erase blocks. Only the final 32 KiB storage window admits array stores and
addressed operations. Other regions and unsupported modes refuse explicitly;
that simulator boundary is not a claim that silicon produces the same error.

Register facts are reviewed against Microchip's
[SAM D5x/E5x datasheet DS60001507F](https://ww1.microchip.com/downloads/en/DeviceDoc/SAM_D5xE5x_Family_Data_Sheet_DS60001507F.pdf),
sections 11.6/11.10 and 25.6.6/25.8. No vendor implementation or register header
is copied. Cache controls do not model cache contents or physical cache timing.
This slice excludes fuses, boot protection, region locks, automatic writes,
ECC, NVM interrupts, auxiliary rows, physical timing and reset persistence.

Manual aligned 32-bit stores populate a page buffer with the documented 64-bit
holding-register transitions and address updates. Main flash stays unchanged
until an admitted WQW command commits its sixteen-byte window. EB erases one
8 KiB block; PBC clears buffering. The key and busy admission, READY, DONE,
PROGE/ADDRE and their supported W1C operations stay distinct. Unsupported
commands refuse rather than being mislabeled as invalid hardware commands.

Four instruction-boundary services provide a finite deterministic busy interval,
not measured silicon latency. Pending operations end Cortex-M batches; the
existing flash-operation predicate also excludes JIT execution. Commit and
READY publication follow in the same boundary, against the actual bus backing.
Default buses take the absent-controller path. Unsupported concurrent array
writes refuse; this does not implement an AHB stall protocol.

## Real-driver guest and mutation qualification

The hosted builder selects codal-samd `62a3a408ac544a4e34bf0beb3932683a2c114e26`,
CODAL core `2ed19d64ec3359143409a4b0a9758b4aeef3ee7d` and framework
`31abf23d118f35010fb75122e60a1eb2b6dffe9f`. It reuses the qualified include-only
ZFlash compiler producer and links that actual object with an authored boot-only
caller, real CPS instructions and freestanding byte-copy/clear routines. It
supplies no invented Flash implementation. This is not a complete CODAL/PXT
runtime or its paired fiber/IRQ service implementation.

The linker reserves the complete storage window outside executable/load data.
The observer declares its initial dirty test block and neighboring canaries
before execution. Only actual guest-driver calls and model operations may
subsequently change them. Six ordered checkpoints require geometry, real erase,
32-byte programming, an eight-byte partial window, actual -1/-2/-3 driver
refusals and re-erase, preserving all neighbors. Single-step observation follows
all four original method entries and LR returns with actual r0 results; a
separate fresh advance-API run checks every checkpoint's backing state. Both
use the declared default peripheral interval of one; no widened-batch result
is inferred. The actual
IRQ helpers must execute three matched CPS pairs in thread context; their
original LR returns and actual entry/return PRIMASK transitions are observed
directly. Startup admits interrupts after data/BSS initialization, establishing
the explicit unmasked, nonnested precondition for these boot-only helpers.

Two isolated copied-model mutations omit erase commit or program commit. Each
must expose its exact actual wrong byte after a real successful driver return;
compiler/setup failures, unrelated errors and exhausted bounds cannot pass.
Both scheduler profiles passed in fresh
[run39113938039](https://github.com/CrispStrobe/labwired-core/actions/runs/39113938039)
at reviewed source `2213923fb9bd1eb5f84aa16a71e7a12fa5a018e4`, tested merge
`83005607fdecc1a5fe018ac9ac483ad43436cb9a` and tree
`b20e8c34d4bec98f8e027bb449b922811042697e`. That tree equals the landed
`7da375fc8ae4f61c191502a542a2d92e6ab9f948` tree; ordered merge parents are
`454058f7cf3a30c8cce684356311ece8c60da49f` and the reviewed source. The test
composition's separately landed C9 diagnostic files establish no C9 result here.
The fresh qualification covers the equivalent Clippy guard correction in the
guest observer while preserving every assertion, driver input and model bound.

Earlier [run39007999563](https://github.com/CrispStrobe/labwired-core/actions/runs/39007999563)
retains its own source `69368aff0cfa07bc4ba6dbbee2967cfceff0010b`, tested merge
`7cec74a855d8b4625e21f0907f04426a0d9f6e9c` and tree
`07462ad5bc05da64610d541866b7c6909cc37098`. Its originals remain historical;
the newer result uses separately admitted once-acquired originals.

Each profile produced twelve checkpoint observations across two fresh execution
modes, ten original ZFlash method returns and six actual CPS helper returns.
Each also detected both exact erase/program commit mutations at their required
wrong bytes after real successful driver returns. Independent readers audited
source/image/commands, sixteen dependency identities, component notices, the
original streams and exact mutation patches. Both profiles used the same ELF
`95a68be3d3e8f4875def40cea1914479b2c36c9ce2bd8226dc64ee0f8500d8dc`. These are finite selected-driver results.
Earlier broader CI failures were retained; the final source uses a peripheral
capability without increasing any downcast ratchet ceiling, and its generated
ignored-test inventory explicitly records the hosted fixture.

## Scoped follow-up contracts

The existing DMA API still borrows caller-owned buffers. Preserve the original
masked async terminal interval, the newer finite unmasked intervals and the
expected premature-release witness; none establishes enforced ownership or
unlimited lifetime. Keep the [qualified DMA boundaries](https://github.com/CrispStrobe/codal-samd/blob/master/docs/PXT-GC-FAILURE-LIFETIME-LANES.md)
when requalifying a new engine.

These are task contracts, not ownership claims. Refresh heads, open PRs and
ownership before starting. Keep admitted sources, notices, original failures,
GC/DMA regressions and all bounds. Completed source-qualified slices are not
permission to replay successful originals.

1. **Consumer engine adoption — complete.**
   [Codal PR72](https://github.com/CrispStrobe/codal-samd/pull/72) adopted the
   explicitly reviewed engine79071f69 with all 22 enabled checks and 21 case
   audits. Historical census/profile identities remain unchanged; unrelated
   default descriptors were not enabled. No browser adoption follows.
2. **Actual RAFFS/settings composition — finite checkpoint complete.**
   [Codal PR74](https://github.com/CrispStrobe/codal-samd/pull/74) qualifies
   actual filesystem/settings with real GC/services, both profiles, 1,871 checks,
   object remount and eight specific mutation/profile cases. Subsequent separate
   contracts cover [compaction](https://github.com/CrispStrobe/codal-samd/pull/76),
   [the current source census](https://github.com/CrispStrobe/codal-samd/pull/77)
   and [modeled retained-backing reset](https://github.com/CrispStrobe/codal-samd/pull/78).
   Inspect each final qualification record. CPU/core-system reset over the same
   Bus leaves peripheral models live; engine restart/backing reopen,
   interrupted operations and power loss require new explicit contracts.
3. **Full board and IRQ/service configuration.** Separately census the remaining
   shared-header dependencies and an actual generated full-board
   configuration. Do not infer USB1 from the service profile or core defaults. Trace required functions to admitted public implementations;
   model and qualify each missing behavior in an owned lane. The boot-only CPS
   helpers here do not qualify full CODAL paired fiber/IRQ semantics. The held
   USB/QSPI stubs and feature-disabled service fixture do not establish the
   original board configuration. Establish actual QSPI source use before
   declaring it required; qualify enabled USB only after its configuration is fixed. Preserve that gap until actual enabled
   configuration and affected guests pass; disabling required features cannot
   close it.
4. **Additional controller semantics or downstream adoption.** Fuses/protection,
   automatic writes, ECC, cache contents/timing, NVM interrupts and reset/
   persistence each need a separate precise model contract and affected guest
   qualification. Native Arcade, WASM/installed Lite and performance require
   their own exact engine/package/consumer evidence after source integration.
   No physical, release or complete storage-runtime conclusion follows from
   the selected flash case.

All source/model changes require affected clean guest qualification and every
enabled exact-head check. Acquire official originals once after metadata
admission and audit retained bytes with standalone readers, without replay.
Documentation-only follow-ups use the applicable source-policy/docs checks;
do not manually dispatch another unchanged ARM guest merely to update prose.
