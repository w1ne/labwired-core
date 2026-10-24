# SAMD simulation-runtime admission lanes — 2026-10-15

This is a prerequisite for original native PyBadge runtime execution and app
adoption, not a prerequisite for authored permissive MMIO guest tests. Keep
[P3 panel binding, P2 IRQ and P4 DMA](target-next-lanes.md) separate. Refresh
default branches, open PRs and consumer lane ownership before implementation.
Do not change the consumer's admission policy to make a historical image run.

## Observed boundary

The consumer's [classification at source 0441d5b](https://github.com/CrispStrobe/brickwright-lite/blob/0441d5b6247e3b3c6a41fac68fba36f0c7387927/overlay/scratch-gui/src/lib/bw-makecode/base-licences.js)
marks both PyBadge base images `chip-restricted`, naming ASF4:

| Build | HEX SHA256 |
| --- | --- |
| CDN | `842c30c5fc1db2346a949837c2e21acdb57937aa0a82f4d505725e995ff02b97` |
| Historical from-source | `9c2310bd5a65f0543c69a076e51a4067c803202a39228f3a7de41451be0da9ca` |

This records the existing registry/policy, not new legal clearance. A reproduced
build or passing preprocessor probe does not make either image emulator-admitted.
[PR174](https://github.com/CrispStrobe/labwired-core/pull/174)'s focused
[run37745531829](https://github.com/CrispStrobe/labwired-core/actions/runs/37745531829)
reproduced the historical build; it did not execute or upload the firmware.
PR174 and PR175 are merged. The qualified separate ARM preprocessing observed
`USE_RGB444=1`; the [original-map receipt](../receipts/2026-10-08-pybadge-link-map/README.md)
records its exact source/configuration boundary. Loaded CF2/constructor selection
remains separate.

## Authored foundations checkpoint — not runtime admission

The following engine-side controls have landed. They qualify small owned
fixtures, not the historical PyBadge image, an integrated CODAL driver or an
application package. The fixture sources remain separately pinned own-fork
prototypes; their use here does not mean their runtime PRs have merged.

- [PR189](https://github.com/CrispStrobe/labwired-core/pull/189) landed the
  authored abstract transfer-state guest. Its events are software state
  transitions, not observed SPI, DMA, IRQ or fiber activity.
- [PR190](https://github.com/CrispStrobe/labwired-core/pull/190) landed as
  `8e038c11a2a341271e0aeafb8fce7d9da23f2c88`, with reviewed and landed tree
  equality. The source-bound [new-base guest run37955448231](https://github.com/CrispStrobe/labwired-core/actions/runs/37955448231)
  passed both default and event-scheduler profiles: positive `0x600d` in315
  machine steps and the restore-always-enabled mutant's exact assertion3 in126.
  Both observed BSS clearance and fixture entry. An independent original
  artifact/metadata audit passed; all19 enabled final PR checks and the
  [full release ratchet37955516120](https://github.com/CrispStrobe/labwired-core/actions/runs/37955516120)
  passed. This is PRIMASK restoration evidence, not IRQ delivery or DMA ordering.
- [PR193](https://github.com/CrispStrobe/labwired-core/pull/193) landed the
  Nordic/F1 Tier1 console-initialization repair and its exact reproduced
  fixture bytes as `80823eb479e20327d8ecb3f35b4d0aa4912a1a2a`.
  [Guest run37946576833](https://github.com/CrispStrobe/labwired-core/actions/runs/37946576833)
  and [full ratchet37946567305](https://github.com/CrispStrobe/labwired-core/actions/runs/37946567305)
  passed. This repaired stale fixtures without changing the engine, ratchet
  thresholds, drift acknowledgements or dependency pins. Preserve PR190's
  [original failed full run37913923779](https://github.com/CrispStrobe/labwired-core/actions/runs/37913923779);
  the repair does not erase that failure or establish a speed improvement.
- [PR191](https://github.com/CrispStrobe/labwired-core/pull/191) landed as
  `428ef1a3f965ab3b641e4ee7be5b03c984a505ed`, with reviewed, actual guest
  and landed tree `7cd052eb5ac39ce7434a0bdf96cb29b796c8067e` equal.
  Its original [IRQ run37939113526](https://github.com/CrispStrobe/labwired-core/actions/runs/37939113526)
  passed on the old base, but is not new-base clearance. After explicit adoption
  of landed PR190, reviewed head `ba979e89889f3e53ffd7068f0ea0a055d01f1fe9`
  passed the [fresh guest37966274198](https://github.com/CrispStrobe/labwired-core/actions/runs/37966274198)
  and [full Core run37966268148](https://github.com/CrispStrobe/labwired-core/actions/runs/37966268148).
  Both guest profiles observed positive600d/count1/stage2 in1501 machine steps
  versus exact premature-delivery assertion6/count1/stage1 in868, with vector,
  BSS, entry and actual exception16 entry/return witnesses. The separate
  original artifact/metadata audit, all19 enabled PR checks and all13 enabled
  full-run jobs passed, including the release ratchet. Preserve the
  [first vector-symbol failure37917350825](https://github.com/CrispStrobe/labwired-core/actions/runs/37917350825)
  and its test-only defined-address-zero fix. No production loader changed.
  This fixture pins CODAL source `2a991760339fe89f5272f923baf89dc18fd434d0`
  and is software-pended IRQ0,
  not actual SAMD peripheral IRQ routing or DMA/fiber completion.

- [PR198](https://github.com/CrispStrobe/labwired-core/pull/198) landed the strict
  paired-policy fixture with ten actual cases across both profiles and all
  enabled checks passing; see its [originals](../receipts/2026-10-11-samd-paired-irq/README.md).
- [PR200](https://github.com/CrispStrobe/labwired-core/pull/200) landed the actual
  upstream Timer diagnostic caller as `f45d0069c042456333ee472a90bf1d1e9e94c1e6`,
  after all 28 enabled checks passed. Both positive and six refusal guests
  passed on the final refreshed base, with separate original artifact audits
  and reviewed/tested/landed tree equality. Read the
  [final originals and exact limits](../receipts/2026-10-12-samd-timer-integration/README.md).
  CODAL PR13 landed the opt-in fixture; the default build and app pins did not
  change. Diagnostic clock/component/allocation/event services are not actual
  runtime service integration.

Next execute the ordered foundation/integration tasks in the
[replacement plan](samd-runtime-replacement-plan.md#next-bounded-foundation-and-integration-tasks).
None of these results moves an app pin, admits a whole runtime, qualifies a
physical board or establishes active PyBadge/native/WASM RTx.

## Actual service integration checkpoint — 2026-10-15

Later, separately selected actual Timer/MessageBus/Fiber/Heap callers and the
aligned public allocator have landed; the older diagnostic fixtures above are
not relabelled as those results. Actual SPI callers now qualify success,
cancellation/recovery, MCLK/GCLK-gated timeout cleanup, writeback AHB error,
software-pended idle IRQ dispatch and bounded blocking terminal-result ownership.
Read the [blocking-result qualification and exact source/image bindings](samd-real-spi-lease-qualification.md).
Runtime [PR32](https://github.com/CrispStrobe/codal-samd/pull/32) and engine
[PR237](https://github.com/CrispStrobe/labwired-core/pull/237) landed after both
runtime and all23 enabled engine checks passed, including all workspace shards
and final aggregates. Four final-source lease/positive cases and a frozen
independent original-log audit passed both scheduler profiles. No driver/model
repair, consumer pin or performance threshold changed in this caller slice.

The later [actual destruction-policy and idle-reconstruction checkpoint](samd-real-spi-lifetime-qualification.md)
now landed through Runtime34/Core239: all ten actual cases and a frozen independent
audit pass both profiles, and both Runtime/all23 engine enabled checks pass.
The earlier conditional-site ratchet failure is retained; the corrected harness
uses the existing central scheduler query without raising the ceiling.

This is not a complete no-ASF runtime or native Arcade image. Keep the selected
Timer, MessageBus, driver and buffers alive. Active-token, delivering-callback
and unconsumed blocking-result destruction deliberately fail-stop; only settled
idle reconstruction completes new transfers. Do not assume destruction cancels
safely. Next complete R1a's
required board/API/header/build closure, R1d's remaining services and R3's exact
component/notices/map/image/CF2 admission, actual frames/input/debugger and
consumer qualification. An excluded required service still blocks the image.
Natural stale IRQs/timeouts, physical behavior and distribution clearance are
separate. Only after admitted active execution measure repeated native/WASM RTx,
adopt the exact installed consumer artifact and optimize a measured bottleneck.
Historical image classifications and the legacy default build remain unchanged.

## R0 — capture actual linked provenance

**Reached:** PR177 captured and independently audited the original map; see the
[source-bound receipt and six-member retained inventory](../receipts/2026-10-08-pybadge-link-map/README.md).
Do not repeat acquisition to obtain an already retained map. Remaining R0 work
includes complete component/header/inline/transitive-script provenance, not
reimplementing the collector. The zero-count members are not clearance claims.

**Dependency follow-up:** the [source-bound dependency receipt](../receipts/2026-10-08-pybadge-dependencies/README.md)
preserves two collector failures, the four surviving shared original rules and
the successful separate `-M` probes of all 172 observed C/C++ commands. The
diagnostic inventory has 542 unique dependencies; one assembly command remains
explicitly excluded. Do not mistake this for recovered original invocations,
retained-inline attribution or completed component review. PR179/180 are now
merged; the [origin receipt](../receipts/2026-10-08-pybadge-origins/README.md)
records PR182's separate hosted source-identity result: 438 pinned Git blobs,
67 original-request files and one captured generated header. All 438 blob IDs
also match the six official pinned Git trees in a separate read-only audit.
PR182 is merged with tested/landed tree equality. The [notice receipt](../receipts/2026-10-08-pybadge-notices/README.md)
records merged PR183's 409 unclassified comment candidates, 133 absent comments,
five repository notices and three installed toolchain package records. Two GCC
package notices cover packaging only and reference additional notices, which
remain uncaptured. Toolchain obligations, exact notice review, generated
derivation, assembly and retained-inline attribution remain open.

The source-bound map-family inventory in that receipt scopes atomic, SPI, ADC,
I2C, UART-helper, SERCOM-handler and table contributions. It is not a call graph
or runtime cost profile; replacing SPI alone cannot close whole-image provenance.

Inputs: exact original build request, immutable dependency commits, compiler
identity and the clean hosted build. Retain the actual link command, linker map,
linker scripts, generated configuration, binary hash and dependency notices.
The first build-evidence artifact lacked a linker map; PR177 now retains the
original clean build's existing map without relinking. Do not manufacture maps
from source lists. If a future diagnostic relink is required, name it separately and
verify the output identity rather than calling it the original invocation.

The pinned [codal-samd build list](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/CMakeLists.txt)
selects `asf4/samd51` and lists eight ASF4 implementation inputs:
`hal_atomic.c`, `hal_spi_m_sync.c`, `hal_usart_async.c`, `hal_adc_sync.c`,
`hal_i2c_m_sync.c`, `hal_io.c`, `hpl_sercom.c`, `hpl_adc.c`.
These are compile inputs, **not** an inventory of retained image bytes.
Headers, inline code, startup, scripts, tables and transitive sources require
separate review even when a map contains no matching object name.

Files: extend the focused hosted evidence collector and add a source-bound
receipt, without running/disassembling the restricted whole firmware.

Acceptance: independently audited allocated input sections/object members,
discarded sections distinguished from retained ones, an exact source/component
and notice inventory, explicit unknowns and reproducible artifact hashes.
Unknown members fail admission; a filename allowlist alone is insufficient.

## R1 — choose the smallest justified permissive replacement boundary

The [source-bound replacement plan](samd-runtime-replacement-plan.md) splits
this lane into reviewable implementation tasks. Its caller observations are
not a completed ABI inventory or replacement implementation.

Inputs: R0, exact API/ABI and register contracts. The pinned
[ZSPI](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/src/ZSPI.cpp)
uses `spi_m_sync_init/enable/disable/set_baudrate`, a SERCOM trigger seam and
DMA completion. The pinned
[DmaInstance](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/src/DmaInstance.cpp)
depends on `SAMDDMAC`. Those observations are clues, not a complete call graph.
Inventory clocks/cache, GPIO, startup/CMSIS, interrupts, timers, UART, ADC/I2C,
USB and storage contributions before selecting replacement slices.

Work only in our own runtime forks. Prefer exact reviewed permissive components
or attributed independent implementations with recorded origins. Review each
component/build and preserve notices/obligations; do not assume every header in
a vendor directory shares one licence, relabel copied code, or claim whole-tree
clean-room origin. For this selected simulation-runtime build, use reviewed
MIT/BSD/Apache components, LGPL components with their applicable obligations,
or GPL library components with an applicable linking/runtime exception and
retained obligations. Exclude ASF4 and vendor/hardware-use restrictions.
An exception's presence elsewhere in a source archive does not establish that
it applies to a selected object. This selection is not blanket licence or
store clearance and does not admit the historical restricted images.

Acceptance: a reviewed source/build plan with immutable inputs, API scope,
excluded modes/devices, affected files, origin records and independently authored
positive/negative guests. The target is a simulation-only runtime, not firmware
approved for flashing or a silent substitution for the historical build.

## R2 — implement and qualify incremental driver slices

Land separately: startup/clock/GPIO foundations, polling SPI, then IRQ and DMA
completion, then other required services identified by R0/R1. Coordinate with
engine P2/P4 rather than fabricating hardware completion in the runtime.
Unsupported services must reject explicitly, not return invented success.

Acceptance per slice: clean hosted source build, exact image/map/source hashes,
authored permissive guest through real MMIO, meaningful disabled-clock/mux/
masked-interrupt/reset/error controls, all enabled final-head checks and retained
attribution. Add a separately named image/request profile; never overwrite the
historical restricted image's hash or claim binary equivalence after replacement.

## R3 — qualify admission, original runtime and consumer adoption

The consumer's current [firmware gate](https://github.com/CrispStrobe/brickwright-lite/blob/0441d5b6247e3b3c6a41fac68fba36f0c7387927/scripts/makecode/firmware-licence-gate.mjs)
is Nordic-oriented. Extend an explicitly SAMD-aware policy only after reviewed
component evidence exists. Tests must reject retained ASF4/vendor-restricted
contributions, unknown objects and changed/unbound image bytes. Passing a Nordic
gate does not certify this SAMD build. Preserve rejection of historical images.

Acceptance: an exact new source/map/binary-bound admission receipt and all
obligations/notices for distributed components; then original-runtime boot,
effective loaded CF2/path selection, frames, input and debugger evidence through
the qualified engine. Only afterward measure active native/WASM RTx and qualify
an explicit app artifact/pin. Do not infer store approval, physical safety,
module colour parity or complete-board support from source or map checks.

Until R3 passes, the existing PXT/translated Arcade route and authored permissive
guests remain distinct usable paths; neither is proof of native-runtime support.
