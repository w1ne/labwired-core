# SAMD critical-section integration boundary — 2026-10-10

Status: pinned interface/source/map review, not an implemented replacement,
complete caller graph, whole-runtime admission or new guest qualification.
The [qualified foundations](samd-permissive-runtime-lanes.md#authored-foundations-checkpoint--not-runtime-admission)
and [replacement plan](samd-runtime-replacement-plan.md) remain authoritative.

Later bounded checkpoint: [original paired-IRQ guest qualification, 2026-10-11](../receipts/2026-10-11-samd-paired-irq/README.md)
records ten passing authored ARM cases, not actual caller or scheduler
integration. Its prospective contracts do not replace the real-consumer
acceptance criteria below.

## Actual definitions and ABI

The pinned [CODAL core header](https://github.com/lancaster-university/codal-core/blob/312ae57e0b31f5b9df07a81e9d846945828e3c5a/inc/core/codal_target_hal.h)
declares two C-linkage, no-argument, void-returning operations:
`target_disable_irq` and `target_enable_irq`. Header blob:
`835c036a6491aebe4f3f26241fc9eaddfa64f590`.

Their actual [ItsyBitsy M4 definitions](https://github.com/lancaster-university/codal-itsybitsy-m4/blob/6ecd80ccf1abcc126d06653f178ca03a4d1c6421/src/codal_target_hal.cpp)
use a static `int8_t` nesting counter. Disable masks interrupts and increments
the counter; enable decrements it, clamps nonpositive values to zero and
enables interrupts. Those definitions do not capture the entry PRIMASK.
Definition blob: `60f3bc383ab72d878fde0cede3dbab6417143784`.

The already retained [original-map receipt](../receipts/2026-10-08-pybadge-link-map/README.md)
identifies both allocated definitions in
`libcodal-itsybitsy-m4.a(codal_target_hal.cpp.o)`. Map SHA256:
`2322e427353ba623b3efadbccfcef1a2ee50c09322d8016dc05b1b782bdb1f64`.
The definition lives in the board runtime, not `codal-samd`; replacing the
unconnected mask header there cannot by itself replace the linked functions.
The original map was read without relinking, disassembly or firmware execution.

The saved-token foundation instead preserves the prior PRIMASK. Mapping its
restore operation to unconditional enable would lose that property. Conversely,
changing legacy enable-at-zero into strict paired restoration may change boot
or externally masked callers. These are source-inferred compatibility risks,
not observed failures of the original runtime. Establish the intended ABI and
caller contracts before implementation; excessive nesting and unmatched enables
need explicit policies rather than relying on narrow-counter conversion.

## Bounded caller census, not active-call or retained-inline proof

At pinned `codal-samd` source `5bd6b93c219c7e784e885ba2d6812809fb6289a8`,
direct spellings of the paired operations appear in `src/DmaInstance.cpp`,
`src/SAMDEIC.cpp`, `src/ZPin.cpp` and `inc/neopixel.h`. These are DMA descriptor,
callback/event, GPIO and bit-banged output seams, not just SPI initialization.

The retained original request has SHA256
`19efcdc73769fdfdeb51aa215c528bebad59782cbc538f72f4a194326f1f42b1`.
Its seven inputs below also contain direct call spellings. The separate
[dependency/origin records](../receipts/2026-10-08-pybadge-origins/README.md)
retain diagnostic `-M` probes of recorded C/C++ source inputs, not recovered
original compiler invocations or a full source-to-binary proof. Their diagnostic
report SHA256 is `bb2429fc32505adbb50fc790592cd3a23d5728cdc438e7d2c2c7a7eca2746c1e`.

| Request input | Direct spelling lines | Diagnostic source/dependency observation |
| --- | --- | --- |
| `pxtapp/core---samd/hf2.cpp` | 136,146,152,178,283,317 | One source unit; request/source digest matched. |
| `pxtapp/mixer---samd/melody.cpp` | 380,394,405,417 | One source unit; request/source digest matched. |
| `pxtapp/mixer---samd/melody.h` | 108,123 | Dependency of two units: melody and pointers. |
| `pxtapp/screen---st7735/panic.cpp` | 272 | One source unit; request/source digest matched. |
| `pxtapp/screen---st7735/jddisplay.cpp` | 226,229,234 | One source unit; request/source digest matched. |
| `pxtapp/settings/SAMDFlash.cpp` | 164,172 | One source unit; request/source digest matched. |
| `pxtapp/settings/RP2040Flash.cpp` | 29,31,39,41 | One source unit; request/source digest matched. |

Presence in the request or a source probe does **not** prove the spelling survives
preprocessing, is allocated/called, or applies to the selected SAMD execution
path. In particular, do not treat the RP2040 filename as either active SAMD
behavior or excluded input without checking its actual preprocessor branch.
The census excludes unidentified additional dependency callers, indirect or
macro-generated calls, and assembly. Those unknowns block a complete ABI closure.

A whole-tree direct-spelling scan of public `codal-core` C/C++ and assembly
files at `312ae57e0b31f5b9df07a81e9d846945828e3c5a` found the two header
declarations above and 57 spellings in nine implementation files. This count
includes source text without preprocessing or comment removal; it is not a
count of executed calls. No files were compiled or executed for this scan.

| Pinned `codal-core` implementation | Direct spelling count | Integration concern |
| --- | --- | --- |
| `source/JACDAC/JACDAC.cpp` | 5 | Service insertion has alternative enable paths. |
| `source/JACDAC/JDPhysicalLayer.cpp` | 19 | Flag macros, callback early returns and queue updates; expand macros and establish actual callback context. |
| `source/core/CodalDmesg.cpp` | 2 | Conditional diagnostic logging can itself enter a critical section. |
| `source/core/CodalFiber.cpp` | 9 | Queue operations and allocation, including allocation failure; actual allocator selection and nested calls matter. |
| `source/core/CodalHeapAllocator.cpp` | 7 | Conditional heap/debug paths and allocation failure; establish selected configuration. |
| `source/core/codal_default_target_hal.cpp` | 1 | Weak terminal panic; distinguish it from the selected board definition. |
| `source/driver-models/Serial.cpp` | 2 | Formatted output masks interrupts; its comment mentions ISR context but does not prove actual invocation there. |
| `source/driver-models/Timer.cpp` | 8 | Event insertion calls `triggerIn`, which has its own pair: representative nested-source case. |
| `source/drivers/MessageBus.cpp` | 4 | Event enqueue/dequeue mutation. |

These paths are relative to the [immutable public dependency tree](https://github.com/lancaster-university/codal-core/tree/312ae57e0b31f5b9df07a81e9d846945828e3c5a/source).
Scheduler, allocator, timer, diagnostics and event delivery therefore belong
in the integration review, not just board SPI callers. The tree-wide scan
closes direct source-text discovery for that dependency's scanned extensions,
not macro expansion, generated callers, selected build configuration or any
other dependency. Map retention and active preprocessor paths remain separate
questions; do not admit the whole dependency from this census.

Each of those nine implementation files matches exactly one source unit and
its source SHA256 in the retained diagnostic dependency report. Their recorded
probe flags select `SAMDX1` and `__SAMD51J19A__`. The retained generated
`build/codal_extra_definitions.h` is 1,634 bytes with SHA256
`1d812a7253fb632cc30537cb34141a1394778c845918172ff5cd36e4953727d8`;
it selects `SAMD51=1`, `DEVICE_HEAP_ALLOCATOR=1`, `DEVICE_USB=1`,
`DEVICE_WEBUSB=1`, `CODAL_DEBUG=CODAL_DEBUG_DISABLED` and a 4,000-microsecond
scheduler tick. These are recorded inputs, not recovered original compiler
execution or fully expanded effective macros. In particular, a replacement
profile cannot silently report successful omission of USB/WebUSB required by
this configuration. Review configuration defaults, forced includes, overrides
and conditional bodies before deciding which callers are active.

The display step has alternative early/normal returns with paired enable calls,
so raw token counts cannot establish balance. The panic path explicitly performs
low-level masking and a target disable without a matching return-path enable;
that is intentional terminal behavior, not an automatically repairable imbalance.

## Next implementation contract

The separately named [selected preprocessing diagnostic](samd-selected-irq-preprocessing.md)
passed source-only staging and effective probes for 21 selected units in
[run38122098497](https://github.com/CrispStrobe/labwired-core/actions/runs/38122098497).
The [original receipt](../receipts/2026-10-11-samd-irq-preprocessing/README.md)
records narrowed HF2 paths, absent own-file RP2040Flash/jddisplay spellings and
surviving melody/neopixel inline pairs. It did not repeat the historical build,
complete the caller inventory, prove dynamic calls or implement ABI integration.

1. Inventory the actual selected definition/header and every direct, transitive,
   inline, macro, assembly and generated caller. Bind immutable source/request
   identities, active preprocessor configuration and retained/discarded inputs.
   Review component origins/notices separately. Do not run the restricted image
   to bypass missing source or licence evidence.
2. Choose explicitly between a compatible paired ABI adapter and coordinated
   saved-token/scoped caller changes. Record nesting, already-disabled entry,
   interrupt-handler context, unmatched enable, excessive depth, startup enable,
   early returns and nonreturning panic semantics. Do not silently redefine the
   old symbols or patch the historical image.
3. Rebuild all affected real consumers cleanly in a named simulation profile,
   without ASF include fallback. Unknown or unsupported required services fail
   explicitly; do not link success stubs for omitted USB/audio/flash/ADC/I2C.
4. Qualify the actual chosen implementation and representative real callers
   through source-bound positive/negative ARM guests: initially enabled and
   disabled, nested/early-return cases, software-pended IRQ delivery, terminal
   panic, depth/underflow refusal and no spurious enable. Pin source/toolchain/
   image/map and observed results; compare scheduler and feature-off behavior.
   Previous mask/IRQ fixtures are prerequisites, not this integration's proof.

No DMA barrier, actual SERCOM/DMAC IRQ mapping, callback/fiber completion,
hardware timing, complete runtime, app pin, legal/store clearance or RTx result
is established here. Only afterward continue peripheral IRQ/DMA and admission.
