# SAMD runtime replacement plan — 2026-10-09

Status: proposed R1 integration boundary, with separately qualified authored
foundations; no integrated replacement runtime is implemented or admitted.
Start with the [R0–R3 contracts](samd-permissive-runtime-lanes.md), then refresh
own-fork heads and open PRs. Do not repeat completed evidence collectors or
execute the historical chip-restricted images. Runtime source changes belong
in our own runtime forks; engine changes belong in LabWired. No upstream posts.

The separately bounded [SERCOM SPI split-IRQ slice](samd-sercom-split-irq.md)
has landed through PR208 after fresh final-base qualification and all enabled
checks. Read its [final receipt](../receipts/2026-10-12-samd-sercom-split-irq/README.md)
before repeating work. Full P2 clock semantics, DMA and real-runtime integration
remain later tasks.

## Why not an ASF-compatible SPI shim?

The [original map](../receipts/2026-10-08-pybadge-link-map/README.md) retains
atomic, SPI, ADC, I2C and SERCOM handler/state contributions. Zero allocated
bytes from two named objects do not clear their headers or other builds.
The [dependency and notice evidence](../receipts/2026-10-08-pybadge-notices/README.md)
also needs header/inline and transitive-origin review. Deleting eight C files
while keeping their headers is not a justified replacement boundary.

Prefer a separately named simulation backend at the CODAL class boundary,
with narrow authored register operations and reviewed component origins.
Do not promise binary compatibility with ASF descriptor layouts. All consumers
of changed public headers must rebuild. Preserve CODAL attribution; this is
integration following source review, not whole-firmware clean-room work.

The eight original ASF compile inputs are `hal_atomic.c`, `hal_spi_m_sync.c`,
`hal_usart_async.c`, `hal_adc_sync.c`, `hal_i2c_m_sync.c`, `hal_io.c`,
`hpl_sercom.c` and `hpl_adc.c`. Their source selection and recursive header
exposure are explicit in the pinned [CMakeLists.txt](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/CMakeLists.txt).
The separate header-exposure review is recorded in
[PR184](https://github.com/CrispStrobe/labwired-core/pull/184#issuecomment-6065697868):
85 flagged headers reach 66 of 172 C/C++ units, including 58 non-ASF units.
These are dependency/comment flags, not licence classifications or proof of
retained inline bytes. Do not use the counts as an admission allowlist.

## Pinned caller observations

All sources below are at `codal-samd` commit
`5bd6b93c219c7e784e885ba2d6812809fb6289a8`. They were read as implementation
and interfaces for this plan; no isolated implementation process is claimed.

| Source | Constraint for the replacement |
| --- | --- |
| [ZSPI.h](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/inc/ZSPI.h) | Its public header includes ASF `hal_spi_m_sync.h` and embeds `spi_m_sync_descriptor`; replacing only linked functions leaves a header dependency. Keep the CODAL-facing API while replacing internal state. |
| [ZSPI.cpp](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/src/ZSPI.cpp) | Initialization uses HAL, HRI, pin/clock helpers and direct registers. Even `write()` enters `transfer()` and DMA. `transfer()` arms a fiber event and schedules; it is not a polling implementation. |
| [SAMDDMAC.h](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/inc/SAMDDMAC.h) | Exposes `sam.h`, `DmacDescriptor` references, eight software descriptors and callback types. Device headers and descriptor layout need their own origin/ABI review. Eight software slots do not limit the engine's hardware channel model. |
| [DmaInstance.cpp](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/src/DmaInstance.cpp) | Uses SAMD21/SAMD51 conditional register paths, end-address increments, beat-sized counts and writeback progress. Select SAMD51 explicitly; do not port the SAMD21 channel window by accident. |
| [DmaFactory.cpp](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/src/DmaFactory.cpp) | Aligns descriptor storage, programs BASEADDR/WRBADDR and enables five SAMD51 IRQs. Handler distinguishes transfer error from completion; preserve that distinction end to end. |

`ZSPI::dmaTransferComplete(DmaCode)` currently ignores its argument, waits for
TXC, drains RX, clears errors and invokes a callback or notification. This is
an observed risk to address in the new profile, not a qualified error behavior.
Do not implement DMA errors by calling the success path, or wait forever for
TXC after a failed transfer. Define error/cancellation reporting with the caller
before implementing it. The existing callback type has no status argument;
changing its semantics or adding state requires explicit consumer tests.

## Ordered implementation tasks

### R1a — bind the complete public API and build closure

Own the runtime build selector and public hardware-facing headers first.
Inventory every consumer of the eight inputs and exposed headers using the
already captured dependency graph; record symbols, types, calling conventions,
error codes and direct-register accesses. Review `ZPin`, serial, ADC/I2C,
timers, clocks/cache, startup/vector tables, CMSIS/device definitions, assembly,
linker scripts, generated settings and samd-peripherals independently.
Preserve exact component pins/notices and unresolved entries.

Deliver a machine-readable source/header/consumer manifest and a named
simulation-only build selection with no ASF include-directory fallback.
Compile every affected consumer from clean sources on hosted CI. A missing
header, unknown source or unsupported requested service must fail explicitly.
No stub may silently claim USB, QSPI, audio or sensor initialization succeeded.
Do not claim R1 complete from the five caller observations above.

### R1b — qualify startup, clocks, GPIO and polling SPI

Author minimal guests from reviewed permissive inputs and primary register
documentation. Prove reset/vector entry, initialized data/BSS, GPIO mux,
clock gating and real SPI DATA/TXC MMIO. This polling test backend must be
named separately from original ZSPI: it does not establish DMA/fiber behavior.

Require positive byte-order and transfer tests plus disabled MCLK/GCLK,
wrong mux, reset during transfer, invalid configuration and timeout controls.
Record exact source/toolchain/image/map hashes and actual engine results.
Retain the existing scheduler and feature-off comparisons. No vendor-header
fallback, host-forced completion or historical-image execution.

### R1c — bridge IRQ, descriptors and asynchronous CODAL semantics

Coordinate with [engine P2/P4](target-next-lanes.md#p2--sam-irq-routing-and-clock-semantics-after-p1).
Qualify per-vector mask/clear/reset behavior before DMA. Then prove descriptor
alignment, count/end-address rules, writeback and real SERCOM triggers through
authored guests; DMA completion alone is not final SPI TXC.

Test TX-only, RX-only, duplex, zero-length, size mismatch, buffer lifetime,
allocation exhaustion, abort, invalid descriptor, bus error and clock loss.
For admitted CODAL callers, prove exactly-once callback/event delivery, no
success on error, no notification from stale/reset transfers and correct fiber
wake-up. Define unsupported cases explicitly before claiming this API works.
Do not shrink the engine model to the runtime's eight software descriptors.

### R1d — close the remaining runtime services and admission

Replace or explicitly exclude atomic/critical-section, ADC, I2C, UART/IRQ and
other services found by R1a. An excluded service that the selected runtime
needs blocks whole-runtime qualification; exclusion is not successful emulation.
Rebuild all changed-header consumers, including assembly/startup review, and
reconcile allocated/discarded map sections with the full dependency manifest.

Only then implement [R3](samd-permissive-runtime-lanes.md#r3--qualify-admission-original-runtime-and-consumer-adoption)
for the new exact source/map/image, with changed-byte, unknown-object and
vendor-contribution rejection controls. Keep both historical image hashes
unchanged and rejected. Qualify actual loaded CF2/renderer selection, display,
inputs and debugger separately; active native/WASM timing and app pin adoption
follow successful execution, not notice capture or terminal-spin benchmarks.

## Toolchain evidence is separate

[PR185](https://github.com/CrispStrobe/labwired-core/pull/185#issuecomment-6066200720)
binds installed header-owner packages to exact source descriptors.
[PR186](https://github.com/CrispStrobe/labwired-core/pull/186#issuecomment-6066618120)
captures the three small packaging notice sets.
[PR187's first actual capture](https://github.com/CrispStrobe/labwired-core/pull/187#issuecomment-6067334329)
adds 12 GCC original-source notice candidates, including the root Runtime
Library Exception v3.1 and a distinct `gcc/m2` variant. PR187 has since merged
as `5f37de2c8f6c5143c2f7031caac53edbce50e448`.
Named-file presence is not header correspondence or exception applicability.
Distribution obligations and exact selected component review remain required.

## Next bounded foundation and integration tasks

### Current continuation after actual SPI callers

The [actual service/SPI checkpoint](samd-permissive-runtime-lanes.md#actual-service-integration-checkpoint--2026-10-15)
supersedes the old foundation starting point below. Preserve its original
source-bound guests; do not repeat reached allocator, callback/fiber, timeout,
DMA-error or software-pended idle-dispatch implementation just to read evidence.
Blocking cancellation/result ownership and bounded lifetime now have separate
qualified [actual checkpoints](samd-real-spi-lifetime-qualification.md).

1. Preserve reached Runtime34/Core239 lifetime qualification: active token,
   delivering callback and pending blocking-result deletion deliberately reach
   actual panic `0xc42`; settled idle deletion/free/reconstruction completes new
   transfers. All ten source-bound cases and the frozen reader pass both profiles.
   Keep finite observation limits, live dependencies, expected-panic versus
   success separation, and original ratchet failure/correction. Do not replay
   unchanged guests or claim safe active cancellation-and-delete. A later shared
   driver/model repair requires affected earlier clean caller qualification.
2. Close required board/reset/NVM/serial and other R1a/R1d APIs against declared
   compatible immutable dependencies. Deliver the source/header/consumer manifest
   and explicit no-ASF build selection; compile every affected consumer. No
   vendor include fallback, invented initialization or silently missing service.
   Start with the concrete gaps: selected `target_reset` currently refuses;
   engine AIRCR reset covers core-local state, not complete peripheral reset.
   Qualify real reset reentry before broadening that promise. The historical
   pointer/copy NVM interface differs from the current integer/read interface;
   compile actual board/settings consumers against the declared compatible pair.
   The PyBadge descriptor has no NVM-controller entry and explicit USB/QSPI
   stubs; a plain flash region does not implement NVM commands. Any required
   native-image service still blocks admission until implemented and qualified.
3. Bind exact components, retained headers/inlines, toolchain obligations,
   compiler/map/binary and effective CF2/renderer path, preserving historical
   restricted-image rejection. Qualify the admitted image through actual guest
   frames, input press/release and debugger pause/step/reset, not host frames or
   this small SPI control caller. R3 remains the admission contract.
4. Measure representative active native/WASM RTx, frame rate and input latency
   repeatedly on declared runners with unchanged floors/baselines. Qualify an
   exact installed consumer artifact/pin separately, then optimize an observed
   bottleneck with repeated controlled A/B and affected correctness evidence.
   Natural stale IRQs/timeouts and physical captures remain separate obligations.

### Retained foundation acceptance contracts

Start with the [landed foundation checkpoint](samd-permissive-runtime-lanes.md#authored-foundations-checkpoint--not-runtime-admission).
Do not repeat successful original acquisitions or infer runtime integration
from the small fixture results. Work these tasks in order, with each source
change separately qualified before the next depends on it.

The new-base IRQ control is complete: LabWired PR191 passed fresh
default/scheduler execution, full release ratchet, independent original audit
and all enabled exact-head PR checks before landing with tested-tree equality.
Preserve its original vector-zero failure and old-base success; do not replay
successful unchanged fixtures merely to read their receipts.

1. **Select the runtime foundation base explicitly.** Refresh the stacked
   [CODAL source PRs](https://github.com/CrispStrobe/codal-samd/pulls) and own
   default branch; the engine's pinned fixtures do not clear those PRs for
   landing. Review their origin/notices and changed files, choose a declared
   simulation-only source base, and clean-build affected consumers on hosted
   CI. Keep authored abstract transfer events separate from real peripheral
   completions; unsupported services must reject rather than report success.
2. **Preserve the reached critical-section caller checkpoint.** The strict
   paired-policy fixture and actual upstream Timer diagnostic caller have landed
   through PR198/PR200. Read the [final Timer qualification](../receipts/2026-10-12-samd-timer-integration/README.md)
   before repeating work: both positive and six refusal guests, original audits
   and all final-head gates are complete. This does not close actual clock,
   component, allocator, event bus or Fiber/Heap integration. Bind the
   qualified PRIMASK primitive to the actual selected caller/build closure,
   starting with the [pinned definition, ABI and bounded caller census](samd-critical-section-interface.md),
   without ASF header fallback. Prove nested/already-disabled restoration,
   meaningful failure controls and the affected real consumer in a clean
   guest. The existing mask fixture is a prerequisite, not this integration's
   acceptance test. No NMI/HardFault, DMA memory-ordering or fiber proof follows
   from PRIMASK alone.
3. **Return to engine P2, then P4 and R1c.** Use primary SAMD51 definitions
   for separate SERCOM/DMAC vectors and pending/mask/clear/reset behavior.
   After actual peripheral IRQ guests pass, qualify descriptors, triggers,
   writeback, error/abort and final SPI TXC. Only then bridge exactly-once
   CODAL callback/event/fiber behavior. Software-pending IRQ0 and abstract
   transfer-state success cannot substitute for these observations.

   **First bounded IRQ slice — reached:** PR208 derives the exact SAMD51 SPI
   flag-to-vector grouping from primary definitions, with SERCOM4 IRQ62–65
   independently masked and cleared. Preserve its qualified contracts and
   original receipts. Reuse `Peripheral::irq_line_levels` where appropriate;
   do not collapse the four lines onto one configured IRQ. Audit both MMIO
   write and read reconciliation: DATA consumption may clear RXC, and the
   existing read-clear hook covers only a single configured line. Enabling
   an already-set flag while idle must deliver the correct IRQ without
   restoring an idle per-cycle walk. Test simultaneous flags, pending-on-enable,
   INTENCLR/W1C/reset/read-clear, NVIC/PRIMASK masks and no stale double entry
   through actual authored handlers. Compare scheduler and forced-walk behavior
   at intervals 1/64/1024; keep clock-gate freeze and idle batching intact.
   Clock-rate derivation, DMA descriptors and real fiber callbacks are later
   separate slices, not success claims of this IRQ work.

R1a provenance/build closure and R1d/R3 admission remain mandatory even when
these foundations pass. No historical image hash or rejection policy changes
are part of this sequence. Active native/WASM performance measurements and
installed consumer/debugger qualification follow admitted execution, not
terminal-spin or abstract-state results.

No firmware admission, complete provenance, legal/store clearance, physical
module parity, active PyBadge RTx, browser result or app adoption is claimed.
