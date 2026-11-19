# CowManager native public Core pin

The native CowManager application can select public Core
`a14fcfb0faa866591baec40c007c723d558dde18` (tree
`d0d038d829bbe45ef32af551671f72d2912c1adf`). It starts from the shipped A1
observer pin `941a6c5caf0c53b66aa5af9290e5ecba5972ec72` and retains all 123
of that revision's top-level chip YAML files unchanged.

This follows [Core #1988](https://github.com/w1ne/labwired-core/pull/1988).
Recognition merge `34dabeccb97e73064eaa6a0817dc560cf9dc3ac0` records the
runtime-tested parent `ed5e27681d991dc581bfbba219a6ecc126d47cf7`. Its tree
exactly preserves main parent `9c180a831f76ea7f6b19f4a428d3a96073db0669`:
`8c3a22b259b7b18dbdb2a0ece2cb0631711ec14c`. Follow-up merge
`31050ec124dfc6cf505f306ab9bd032cf985b4f3` records the released-CLI
fixture-only child. Recognition merge `1566d27a8f25c2decbeede1b9400417cafe48ece`
records runtime-bearing parent `232b279705ba2083dd282e6ff9a7e6efd5bf0973`
with the two validated A1 GPIO repairs. Metadata recognition merge
`fbb397a686dde52dae3f9dae61c939679a13f08d` records the final child. Each merge
preserves its preceding tree. Relative to the main base, only this record
changes. Current main's simulator and model sources remain intact while the
reviewed public pin becomes a mainline ancestor through the protected merge.

## Included source

- BMA400 declarative SPI device and capture example from `37ee11b1c1`.
- The BMA400 and actual-MNA smoke tests from `e2e228b00e`.
- Same-machine bus-trace export from the test command, from `d7b750c3e8`.
- The existing source/capture provenance policy and tests from
  `a0b0a91c1868fe78e4af45e817e566807517ef1e`, including its manifest header.
- The explicit nRF5340 TWIM source-watch entry, following `d9ccc2624e`.
- Canonically generated BMA400 peripheral-manifest and device-catalog entries.
- Existing A1 follow-up `ff78e6b708` resolves SAMD/AVR port labels through the
  board's pin authority. Its two regressions use this pin's flat test layout.
- Existing A1 follow-up `f57baef72d` delivers autonomous STM32 AF edges to GPIO
  observers through weak wire listeners and the shared route-selection rule.
  Its only production context adaptation adds listener synchronization inside
  the existing SAM-only reset block; later F1 reset/LCKR/IDR work is not copied.
- A released-CLI self-test fixture update from the older STM32L476 pair to the
  existing STM32F103 committed smoke. All five gate controls pass against the
  SHA-verified v0.26.0 CLI, including wrong-marker rejection when the CLI exits
  zero. The real chip map, firmware files and thresholds are unchanged.
  [Issue #2057](https://github.com/w1ne/labwired-core/issues/2057) records the
  older L476 input incompatibility; this fixture choice does not fix it.

The public flash-store API remains byte-identical to the shipped base. No
chip descriptor is added or removed. The metadata follow-up removes only the
redundant APA102 tested entry, following upstream `9b8a55a764`. The static
part-coverage gate now derives that classification from the added LED-system
regression; unchanged APA102 SPI/parity tests remain the behavioral evidence.
All totals remain 127 parts: 47 lab-exercised, 79 test-exercised and one
device-only, with zero gaps. Both `min_lab_exercised: 47` and
`max_device_only: 1` remain unchanged. The app's existing 40-gap browser-coverage
limit also remains unchanged. The previous app candidate,
`ddc48b41198643c937d70c2050fee9a9cb395913`, introduced 31 additional chip
descriptors without exact browser paths; this public pin does not import that
unrelated expansion.

## Independent public and private builds

The public app/WASM/config exporter uses the pin above. The private K32 runner
continues to use Core `ddc48b41198643c937d70c2050fee9a9cb395913` with private
model revision `f92e3b6f006061f97a4470eee480237fcfc00a0c`. Public and private
Core revisions are intentionally independent in the builder recipe. Their
boundary is the system/stimulus input and the run-result/trace formats.

App acceptance must freshly run the final public export through that exact
private engine, record both revisions and the engine/system/firmware/stimulus
hashes, and check the normal default run budget. This public compatibility pin
does not roll back the private runtime. Its production image and deployed
identity remain separate release checks.

## Historical evidence and validation

The A1 base had changed watched GPIO source without changing five older source
acknowledgements. No existing capture recorded a matching source digest for
those exact files. The adopted upstream policy preserves capture and
acknowledgement fields and reports missing capture digests as unverified.
All 60 historical board tiers and capture dates remain intact; the nine
existing capture rows make no current-source-match claim. The nRF5340 change
adds an explicit watch member required by the structural test and retains the
existing recursive directory watch.

The unchanged provenance-policy suite passed 56 tests when adopted. The final
GPIO changes update 37 model-digest cells in the existing generated report;
all 60 tiers/dates and the nine unverified capture labels remain unchanged.
The source identity of the historical ed5 parent is not asserted for this
GPIO-modified source. The final public commit passed normal formatting,
Clippy, host-workspace and generated-report checks.

The same 70 cases passed under Core defaults and under the workspace's
`event-scheduler,jit-framework` graph: zero failures and one unchanged ignored
case in each. Every one of the 44 prior workspace failures passed by name in
both runs. This includes the exact AF and A1 controls, both LED-system
regressions, and complete PadLines/PadRoutes suites. The validated seven-file
source hashes match the final commit. The unchanged part-coverage gate and
all 19 focused coverage tests pass after deleting the stale APA102 entry.
All three exact-head BMA400 smoke cases pass after normal publication.

Relative to runtime-bearing parent `232b279`, the final child changes only the
coverage ledger entry and its engineering note. Runtime source, chip/device/
system configurations, fixtures and build inputs remain byte-identical. The
fresh native execution records retain their 232b attribution; guarded app
WASM/source and export-input equivalence checks cover the metadata child.
Exact-head [Core CI](https://github.com/w1ne/labwired-core/actions/workflows/core-ci.yml)
for `a14fcfb0faa866591baec40c007c723d558dde18` and this recognition PR's
normal protected checks must pass before the app selects the reachable pin. Hardware capture dates
were not advanced, and no hardware validation is claimed by these source tests.

[App PR #3500](https://github.com/w1ne/labwired/pull/3500) tracks regeneration
of the app data and WASM, unchanged browser-coverage gates, private-runner
acceptance, deployment and the saved native-project browser workflow.
[Issue #3497](https://github.com/w1ne/labwired/issues/3497) retains the product
acceptance boundary. This Core record alone is not production-demo evidence.
