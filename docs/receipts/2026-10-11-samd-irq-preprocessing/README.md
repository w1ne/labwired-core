# Selected SAMD IRQ preprocessing — original result

The separate [run38122098497](https://github.com/CrispStrobe/labwired-core/actions/runs/38122098497)
passed on source `eb5395e0abcb4054c2b50057edbf29cfdf2e22e1`, attempt 1.
[PR196](https://github.com/CrispStrobe/labwired-core/pull/196) merged as
`fe003506c6ace2b6e10465a18a67bf147b1fa071`; reviewed, predicted and landed tree
`0451c46160de8f1f0fb22657d9b2d950d3a1c16d` matched. All 15 enabled final-head
checks passed, including both late aggregates. The four declared manual/warm
jobs skipped: `pr-gate-warm`, `core-integrity`, `ci-runner-image`, `core-full`.
No unchanged guest or historical firmware build was repeated for this result.

## Original identity and audit boundary

The sole [official artifact11695708418](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11695708418)
is `samd-selected-irq-preprocessing`, a 4,630-byte ZIP with one member,
`result.json`. Original ZIP SHA256:
`a1d8d64705a539dde377b2b1e7b9ed5842950cf667a31ed54fa8345de3f2ef48`.
The [unchanged original result](result.json) is 49,007 bytes with SHA256
`37ddf9c0803b8a6d03f86446b9302d4a87613a82fb486d3a9701fa563fe1e983`.
Frozen [probe input](../../../scripts/samd-irq-preprocess/input.json) SHA256:
`f504376d05fec3c5963a7f01af035688586f85bd6f6e0bfcfc74047e308165e0`.

A separate standard-library audit, without importing the producer, checked the
official source/attempt/conclusion, sole artifact/member, input and source
identities, all 21 unit results, dependency-match flags, permitted bound spelling
locations, six selected macro values and false admission/selection/execution
flags. It passed. This audit validates the original metadata and result
invariants; it did not independently replay the compiler or reconstruct raw
preprocessed output. Raw vendor-expanded text is not included in the artifact.

## Observed selected paths

All 21 selected units completed separate `-E` and `-E -dM` probes. All 278 unique
dependency bytes and the compiler were bound before/after; each unit's effective
dependency set matched its frozen separate original `-M` observation. Effective
macros in every selected unit included `SAMD51=1`, `DEVICE_HEAP_ALLOCATOR=1`,
`DEVICE_USB=1`, `DEVICE_WEBUSB=1`, `SCHEDULER_TICK_PERIOD_US=4000` and
`USE_RGB444=1`. A replacement must not silently omit USB/WebUSB required here.

The following are own-source spelling locations after preprocessing, not counts
of calls, dynamic execution, retained code or balanced critical sections:

| Input | Observed own-source locations / scope |
| --- | --- |
| CODAL core JACDAC | 83,87,90,101,110 |
| CODAL core JDPhysicalLayer | 264,274,281,287,324,348,354,848,853,884,886,916,918,948,950 |
| CODAL core Dmesg | 128,165; retained despite the selected disabled debug macro |
| CODAL core Fiber | 95,99,123,150,165,179,189,201,211 |
| CODAL core HeapAllocator | 157,169,208,251,272 |
| CODAL core default target HAL | 28; terminal-path disable observation |
| CODAL core Serial | 437,482 |
| CODAL core Timer | 67,69,167,175,192,209,282,302; nested-source integration remains necessary |
| CODAL core MessageBus | 182,200,212,225 |
| ItsyBitsy M4 target HAL | 8,13,17,19,75; includes definitions and low-level operations, not five calls |
| SAMD DmaInstance | 99,113,142,207 |
| SAMD EIC | 55,57,101,103,112,115 |
| SAMD ZPin | 145,167,760,776 |
| Request HF2 | Only 152,178 of the six earlier source-text locations remain |
| Request SAMDFlash | 164,172 |
| Request RP2040Flash and jddisplay | No own-source IRQ spellings observed in this selected preprocessing |
| Request panic | Low-level disable 271/276 and target disable 272; do not invent a returning enable |
| Request melody | Own-source 380,394,405,417; inline header 108/123 in melody and pointers |
| Request light | Inline neopixel header 66/173 |

The full result preserves every observed location and selected macro, including
declarations and low-level helper definitions from bound dependencies. The
absence of own-file spellings is not an absence-of-all-IRQ or whole-image claim.

## Next actual integration boundary

### Separate read-only assembly source review

The excluded public [CortexContextSwitch.s](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/asm/CortexContextSwitch.s)
was separately read at the pinned Git tree, not obtained from the preprocessing
artifact or compiled. Blob `65c05640b95c20f173db5964d66991dd18f665b1` is
7,872 bytes with SHA256
`bf8c9b4e9946168138214e3ae27ac248130ffd3325d4c8f8b2064aa378f692d9`.
Its retained notice identifies MIT terms and BBC attribution. This component
source observation is not whole-runtime licence clearance.

After removing assembler `@` comments, a case-insensitive token scan found no
direct `target_disable_irq`, `target_enable_irq`, `cpsid`, `cpsie`, `primask`,
`basepri` or `faultmask` spellings. The source selects `.cpu cortex-m0` and
provides register/stack context-switch routines; it does not explicitly save
PRIMASK with a fiber context. This closes that narrow direct-token observation,
not assembly execution, instruction decoding, transitive call behavior, context
ownership or compatibility with a new paired-depth adapter. Decide whether
scheduling may occur with an outstanding critical section before assuming a
global nesting state can safely cross fiber contexts.

### Remaining implementation

Use the [interface contract](../../engineering/samd-critical-section-interface.md)
and [runtime replacement plan](../../engineering/samd-runtime-replacement-plan.md).
Review effective nested/early-return/terminal behavior and remaining generated,
macro, indirect and assembly callers. The excluded assembly input is
`libraries/codal-samd/asm/CortexContextSwitch.s`; lowercase assembler input must
not be called preprocessed coverage merely because a default `-E` invocation
returns no text. Establish its actual language/flags and direct mask behavior.

Then explicitly choose a separately named paired-ABI compatibility profile or
coordinated saved-token caller migration. Decide already-disabled entry,
startup/unmatched enable, excessive depth, ISR context and nonreturning panic
before linking real consumers. A strict saved-mask adapter is not a transparent
replacement for the historical enable-at-zero ABI. Qualify clean real-caller
positive/negative ARM guests without ASF fallback. Additional caller coverage,
notice/component obligations, retained-inline attribution, runtime admission,
peripheral IRQ/DMA, app adoption and active PyBadge RTx remain open.
