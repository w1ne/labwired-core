# CowManager native public Core pin

The native CowManager application can select public Core
`e99163ae3a1f1b2e799aa03839048d30dc035dff` (tree
`c7ebc1c5ab8eb55f6d0a9ad0d6d378df884e51d7`). It starts from the shipped A1
observer pin `941a6c5caf0c53b66aa5af9290e5ecba5972ec72` and retains all 123
of that revision's top-level chip YAML files unchanged.

This follows [Core #1988](https://github.com/w1ne/labwired-core/pull/1988).
Recognition merge `34dabeccb97e73064eaa6a0817dc560cf9dc3ac0` records the
runtime-tested parent `ed5e27681d991dc581bfbba219a6ecc126d47cf7`. Its tree
is byte-identical to its main parent `9c180a831f76ea7f6b19f4a428d3a96073db0669`:
`8c3a22b259b7b18dbdb2a0ece2cb0631711ec14c`. Follow-up recognition merge
`31050ec124dfc6cf505f306ab9bd032cf985b4f3` records the final CI-fixture-only child and
preserves its preceding tree, including this compatibility record. Only this
record changes relative to Core main. Current simulator and model sources
remain intact while the reviewed public pin becomes a mainline ancestor
through the normal protected merge.

## Included source

- BMA400 declarative SPI device and capture example from `37ee11b1c1`.
- The BMA400 and actual-MNA smoke tests from `e2e228b00e`.
- Same-machine bus-trace export from the test command, from `d7b750c3e8`.
- The existing source/capture provenance policy and tests from
  `a0b0a91c1868fe78e4af45e817e566807517ef1e`, including its manifest header.
- The explicit nRF5340 TWIM source-watch entry, following `d9ccc2624e`.
- Canonically generated BMA400 peripheral-manifest and device-catalog entries.
- A released-CLI self-test fixture update from the older STM32L476 pair to the
  existing STM32F103 committed smoke. All five gate controls pass against the
  SHA-verified v0.26.0 CLI, including wrong-marker rejection when the CLI exits
  zero. The real chip map, firmware files and thresholds are unchanged.
  [Issue #2057](https://github.com/w1ne/labwired-core/issues/2057) records the
  older L476 input incompatibility; this fixture choice does not fix it.

The public flash-store API remains byte-identical to the shipped base. No
chip descriptor or coverage ledger is added or removed. The app's existing
40-gap browser-coverage limit remains unchanged. The previous app candidate,
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

The full provenance-policy suite passed 56 tests. The final public commit
passed normal formatting, Clippy, host-workspace, and generated-report checks.
[Exact candidate CI](https://github.com/w1ne/labwired-core/actions/runs/44468106541)
and the recognition PR's normal protected checks must pass before the app
selects the reachable pin. Hardware capture dates
were not advanced, and no hardware validation is claimed by these source tests.

[App PR #3500](https://github.com/w1ne/labwired/pull/3500) tracks regeneration
of the app data and WASM, unchanged browser-coverage gates, private-runner
acceptance, deployment and the saved native-project browser workflow.
[Issue #3497](https://github.com/w1ne/labwired/issues/3497) retains the product
acceptance boundary. This Core record alone is not production-demo evidence.
