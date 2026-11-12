# LabWired Core fork mergeback — 2026-11-10

## Result and scope

The fork network contains three repositories with divergent branch history: CrispStrobe, MbedLabs and blue-az. The other 36 readable repositories have one branch each and zero commits ahead of the pinned upstream main. A fork alone is not evidence of active product usage.

- Upstream: `w1ne/labwired-core`, pinned main `3e227a9b3b8332b38d33a721d2662b237a185ebb`.
- GitHub metadata reports 39 forks; the paginated fork collection returned 41 entries. All returned entries are public. Branch enumeration succeeded for 39 repositories. `ayrinajahan/labwired-core` and `rosaliakoch/labwired-core` returned HTTP 404; their contents and state are not established.
- Enumerated 416 branch tips, comprising 359 unique SHAs. Compared every readable tip using `git rev-list --left-right --count BASE...TIP`; zero comparison errors.
- 349 branch tips have history absent from upstream. These are overlapping and often rebased histories, not 349 independent mergeback features.
- Evidence: [complete public branch comparison snapshot](fork-mergeback-2026-11-10.json). Every count is pinned to the snapshot, not the moving branch names.

## Verified downstream use

CrispStrobe's pinned [README](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/README.md) describes LabWired as the firmware engine for Brickwright-lite, through bw-board. It builds a SHA-pinned WebAssembly engine; bw-board adapts circuit netlists to manifests, while Brickwright-lite provides a browser/Tauri circuit and firmware debugger. The stated boundary is MCU registers/peripherals/pads in LabWired and the off-chip circuit solver in bw-board. The document lists micro:bit/CODAL, PyBadge/SAMD51, ESP32-family Arduino and firmware-debugging uses. This is repository evidence of integration; this audit did not run those downstream applications.

At the audited SHAs, CrispStrobe main is 190 commits ahead and 16 behind upstream. Its main is not the consumer release pin. The immutable [producer build recipe](https://github.com/CrispStrobe/bw-board/blob/33d3ee399db9224105e70f34864f1af962ccdb41/scripts/build-labwired-wasm.mjs) and [consumer sync recipe](https://github.com/CrispStrobe/brickwright-lite/blob/d318ed6e22fb774668cbf1a67184415870e5820c/scripts/sync-labwired-wasm.mjs) both pin `fa992f357df88dbd50057a31f13dbcc0835a95cd`. The consumer checks SHA-256 for the WASM, JavaScript glue and software profile. Older pins in the fork README are historical. This audit did not run the downstream application.

## Mergeback preparation and constraints

The replayed CrispStrobe history retains old commit subjects, but the residual patches often changed during rebasing. Do not cherry-pick by subject alone. For example `011c911de` now adds DMAC request latency, while `9bd7d6fad` now adds split-port/MISO SPI routing. The WFI commits `765e3ae79` and `e5d61d3d7` contain tests/docs only; source changes were folded into older `c731d22a0`.

### First narrow candidate: ESP32 duty latch timing

Source: [59dca3daa4b9b6ada5e1d93619ddf1de04985ca5](https://github.com/CrispStrobe/labwired-core/commit/59dca3daa4b9b6ada5e1d93619ddf1de04985ca5).

The current classic ESP32 LEDC model treats every channel `CONF0` write as a timer change. The low-speed ESP-IDF update sequence writes `CONF0` after `DUTY_START`, so this applies a pending duty in the current PWM frame. The primary-source check used [ESP-IDF v5.5 `_ledc_update_duty`](https://github.com/espressif/esp-idf/blob/v5.5/components/esp_driver_ledc/src/ledc.c) and its [ESP32 HAL](https://github.com/espressif/esp-idf/blob/v5.5/components/hal/esp32/include/hal/ledc_ll.h): the low-speed update sets `conf0.low_speed_update` after `conf1.duty_start`. The port compares the old and new `TIMER_SEL` bits. An unchanged timer keeps the new duty pending until overflow; a real timer change retains the existing immediate rebind behavior.

Only the LEDC hunk is selected. The same source commit removes a duplicate S3 ROM thunk introduced by the fork's merge; that cleanup is not needed upstream. Added tests cover native word writes and four byte writes in both speed banks. They assert actual pad levels before and at overflow, self-clearing `DUTY_START`, and the real timer-rebind control.

### First narrow candidate: preserve I2C stretching through wrappers

Source: [523dcbca32ec3e2b1f4514dad5a8f389fedc6568](https://github.com/CrispStrobe/labwired-core/commit/523dcbca32ec3e2b1f4514dad5a8f389fedc6568).

Current upstream already has the `I2cDevice::clock_stretch_ns` capability and a SAMD51 SERCOM timing consumer. The trace decorator, supply decorator and TCA9548A mux omit forwarding, so they replace a device's hold with the default zero. The prepared patch adds 20 production lines and three regression tests. It preserves live supply cutoff and uses the maximum hold among selected enabled downstream devices. It fixes misplaced method documentation present in the fork patch.

The first mergeback batch ports only the three wrapper methods and adds `crates/core/tests/i2c_clock_stretch_wrappers.rs`. The existing generic trait is sufficient. No new device-specific tool, schema or simulator is needed. Runtime verification is recorded below after testing against the pinned upstream.

The timing test executes the real SERCOM model. At 48 MHz, a payload's nine 10-cycle periods plus 25 us stretch require 1290 cycles. It must remain pending at 1289 and deliver the byte at 1290 for direct, trace, supply, mux and composed wrappers. Other cases cover a live supply cutoff/restart, a live change in hold duration, independent addresses, disabled channels, control-address traffic and maximum rather than first/summed stretching.

Existing limit: SERCOM computes address-phase duration before calling `select_address`, so a mux can report its prior transaction selection during that phase. This patch verifies payload timing and does not claim to repair first-address timing. This deserves a separate controller admission test and fix.

### Next candidate: generic WFI parking and shared NVIC wake

Source tree: [Cortex-M at d543a46b](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/crates/core/src/cpu/cortex_m.rs). Supporting commits: `c731d22a0a7bccadec0916220d277c80ae1f95ab`, `30461884799abdb24bed5d2ceeac95c3761df337`, tests `765e3ae79f9f32b5ad91693eb2938dbacb5260fb` and `e5d61d3d770ae094f0a53dc4c56e81c4f33070c1`.

Upstream unconditionally clears the sleep state at an ordinary step after a refused idle skip. The fork retains WFI parking while active bus work needs service. It also checks enabled shared NVIC pending bits before WFI/WFE idle admission, avoiding a wake being missed until the next import boundary. A separate generic port adds shared-NVIC eligibility and preserves parked WFI while the bus needs service. The initial new suite on unchanged upstream reproduced three failures: shared wake admission, cleared/disabled shared wake admission, and a CPU that fetched beyond WFI during 37 peripheral service cycles. The existing and spin controls passed (8 passed, 3 failed overall).

Review then found two gaps that a direct fork copy would retain: warmed native/browser JIT blocks bypass the interpreter sleep check; and WFI can treat already-imported but now cleared/disabled IRQs as wake events. The prepared port covers those paths separately. Do not pull the SAMD-specific chunk executor into this correctness fix.

`cortex_m.rs` is watched by seven existing silicon-capture records. `validation/manifest.yaml` requires human acceptance of the exact changed content or a new capture; simulation tests cannot renew those records. This first batch does not change CPU source or the acknowledged digests. The [prepared WFI patch and evidence](mergeback/cortex-m-wfi.md) remain a separate review item. Its corrected default suite passes 13/13; native JIT plus the event scheduler passes 14/14. A deliberate native negative control fails the warmed-cache and stale-pending tests (12 pass, 2 fail). Browser execution and performance are not established by those results.

### Hold the broad SAMD execution bundle for an independent review

The fork adds an explicit no-ASF SAMD simulation profile (`550ae080932363dd2c915c1b599e6642b08a63e2`) and checked memory-only execution chunks across CPU, bus and machine dispatch. It is substantially larger than the WFI fix and has a fleet performance tradeoff. [82cbad324fece1dba99e6f7df97d2b85b0c3b5bf](https://github.com/CrispStrobe/labwired-core/commit/82cbad324fece1dba99e6f7df97d2b85b0c3b5bf) explicitly raises fork step baselines after measuring approximately 19 extra host instructions per Cortex-M step. [4da3915f9fae034fda2ad3a1bdb158b5fb79799b](https://github.com/CrispStrobe/labwired-core/commit/4da3915f9fae034fda2ad3a1bdb158b5fb79799b) supplies ARM64 disassembly and attributes the x86 cost to layout; that causal claim is not independently established by this audit. Do not import rebaselines as correctness fixes. Require the unchanged upstream performance gates on an isolated extraction.

### SAMD peripheral follow-ups need hardening

- SPI split-port/MISO routing (`9bd7d6fade402743461dce9bacc47cd36b110d82`) must include matching factory/config construction and tests for unselected DIPO or disconnected MISO. Its historical subject does not describe its current patch.
- DFLL/USB clock-tolerance additions (`dcbf1322d1878ce580a7e48c2131df6ac7d09321`, `eaa3812ed1ac253b0cc552548f88b8737e60eeb4`) accept an unrestricted signed `dfll_open_loop_ppm` then call signed `abs()`. Source inspection finds an `i64::MIN` overflow path; use a bounded input or unsigned magnitude and add edge-value tests before adoption. The factory also silently converts malformed values to zero.
- DMAC request latency (`011c911de6898de51e4be460f9e36aebe7d9d439`) accepts `u64` latency and evaluates `at + latency` without overflow handling. Its `seen` timestamp is not cleared on a missing request; verify cancellation/reassertion timing and channel disable/re-enable, then use safe elapsed-time arithmetic before adoption. These are source review findings, not executed reproductions.
- NVMCTRL committed-operation counters and pending-operation observation are useful for power-cut testing, but should be extracted independently from quiescent-executor changes and checked across reset and aborted operations.

## Further useful work and its admission checks

See the [next-port source map](fork-port-map-2026-11-10.md) for the exact sensor and RGB commit slices, hidden registration hunks, existing PR overlaps and acceptance tests.

| Slice | Why it helps LabWired | Required evidence before adoption |
|---|---|---|
| Sensor data-ready and open-drain outputs | Shared interrupt lines and motion sensors work through the existing pin model. | Port the generic opt-in schema and scheduling behavior together; run source-built TWIM/GPIOTE guests with the event scheduler both on and off. Check first edge, shared-line release, disabled conversions and reset. |
| ESP32-S3 LCD_CAM / GDMA RGB panel and GT911 touch | Extends browser HMI simulation to the ESP32-4848S040 class of panel. | Isolate engine models from fork records; verify DMA addressing, RGB scan-out, touch transactions, high-bank pads, disconnects and reset. Use existing generic display/input APIs. |
| Browser pad release and replay | A circuit solver can release an external pin and restore the same experiment. | Evaluate the existing generic pad/input surface first. If an API extension is needed, include strict numeric validation, unbonded-pad rejection, release behavior and snapshot replay together. No new MCP tool is needed. |
| Realistic-workload hang guard | Prevents a stuck guest from being recorded as a faster simulator. | Reproduce the stuck-ROM negative control and measure instruction/cycle bounds on upstream firmware. Do not copy the fork's numeric performance baselines or relax existing gates. |
| SAMD native game execution | Runs more real firmware at interactive speed. | Keep the explicit software profile separate from claims about silicon. Qualify generic execution changes on the unchanged upstream performance fleet before adopting any acceleration. |

The present upstream already exposes ADC input injection and release, logic capture, routing inspection and manifest clock selection. Historical downstream bridge notes do not establish current defects in those surfaces. The external circuit/MCU boundary is a useful integration pattern; it does not justify a second engine or device-specific MCP interfaces.

## Other forks and consumers

`blue-az` used core for shared-memory I2C co-simulation, a VL53L1X time-of-flight lab and trace/audit checks. The substantive contributions are already merged as [#87](https://github.com/w1ne/labwired-core/pull/87), [#308](https://github.com/w1ne/labwired-core/pull/308), [#491](https://github.com/w1ne/labwired-core/pull/491) and [#1071](https://github.com/w1ne/labwired-core/pull/1071). Its old device-specific distance setter should not return: current `SimInput`, per-device identity, YAML `distance` and Python `Session.set_input` cover the use case. Its executable-fixture race workaround is superseded by upstream #909.

MbedLabs main contains no commits ahead of the pinned upstream. Many branches in MbedLabs and blue-az preserve old upstream feature histories whose changes were squash-merged or replaced. Commit novelty alone does not make them mergeback candidates. An old browser JIT branch also specializes fixed program counters in one firmware image; use the generic upstream adapter instead of importing those assumptions.

First-party integrations mainly consume releases or public APIs:

| Consumer | Verified integration | Action belongs to |
|---|---|---|
| [firmware-test](https://github.com/LabWired/firmware-test/blob/be55bcf08b6ee2e3f5858bdccf3a2060e02a99e4/action.yml) | Installs a release CLI; default `v0.21.0` at this snapshot. | Release compatibility and version updates. |
| [firmware-ci-starter](https://github.com/LabWired/firmware-ci-starter/blob/1c951545b3acc11aaadb027fbf8adbf67d9a1f5a/.github/workflows/labwired.yml) | Calls the firmware-test action. | Action contract tests. |
| [labwired-zephyr](https://github.com/w1ne/labwired-zephyr/blob/daa43fc14d8c3f7cd3681f56d0db198ed7293d04/README.md) | Calls an installed CLI with a system manifest. | CLI/manifest compatibility. |
| [labwired-vscode](https://github.com/w1ne/labwired-vscode/blob/041181dfeb945ceb0b1007ea414fce9c93aeaf4a/package.json) | Downloads CLI/DAP; default `v0.25.0` at this snapshot. | Release/DAP compatibility. |
| [winspeedrun](https://github.com/17prabhanshu/winspeedrun/blob/e396f20d226627a1e88a61c8a811f608dbd9931b/UPSTREAM.md) | An additive CLI/Python SDK layer; documents an unmodified simulator. | Consumer API tests, not engine mergeback. |

## Verification of the first batch

Baseline at `3e227a9b3b8332b38d33a721d2662b237a185ebb` with only the new tests:

- PWM negative control: the latch clears immediately after a same-timer word write, so the new regression fails (0 passed, 1 failed).
- I2C negative controls: all three new tests fail. Tracing completes the payload before SCL release; the mux returns 0 instead of 75,000 ns; nested tracing/supply returns 0 instead of 25,000 ns.
- WFI candidate controls: 8 passed, 3 failed, as described above. Those candidate tests are separate from the first batch.

Corrected-source results on 2026-11-10:

| Check | Result |
| --- | --- |
| Classic ESP32 LEDC suite | 29 passed, 0 failed. |
| New I2C wrapper integration suite | 3 passed, 0 failed. |
| Existing TCA9548A suite | 13 passed, 0 failed. |
| Existing bus-trace suite | 18 passed, 0 failed. |
| Existing shared-supply integration suite | 6 passed, 0 failed. |
| Repository pre-commit gate | Passed: formatting, Clippy with warnings denied, classified host build and validation drift gate. |
| Repository default pre-push gate | Passed: Clippy, event-scheduler walk deletion and RP2040, LogicTap, unknown peripheral types, nRF52 and descriptor integrity. |
| Generated model ratings and firmware exercise matrix | Both `--check` commands passed. |
| Whitespace validation | Passed for the final staged changes. |

The first batch used Rust 1.95.0 with the repository's optimization level 1, debug information disabled, one build job and incremental compilation disabled. The core suites used `cargo test -p labwired-core`: `--lib peripherals::esp32::ledc::tests`, `--test i2c_clock_stretch_wrappers`, the existing TCA9548A and bus-trace library filters, and `--test shared_supply_domains`. The normal repository hooks also ran during commit and push; no hook was bypassed. This is targeted verification, not a claim that the full workspace suite ran.

Upstream advanced during the run to `75700134a3b31124bb46eae118acc67d65b4b546` through CI-only PR #1818. The only added upstream path was `.github/workflows/core-workflows-lint.yml`; the tested source remained identical. The inventory and negative controls remain pinned to `3e227a9b3b8332b38d33a721d2662b237a185ebb`.

Performance baselines and hardware drift acknowledgements remain unchanged. The generated validation-status update changes only the classic ESP32 simulation model digest.

## Complete readable fork table

Ahead/behind counts are default-branch history relative to the pinned upstream. The divergent-branch count includes historical branches and does not imply patch novelty.

| Fork | Branches | Branches ahead | Default ahead / behind | Default SHA |
|---|---:|---:|---:|---|
| [Aquahoodjd/labwired-core](https://github.com/Aquahoodjd/labwired-core) | 1 | 0 | 0 / 1 | `841be9961e8e718f5c0f11ded698e1ccf7f81a0e` |
| [balamurali13-bot/labwired-core](https://github.com/balamurali13-bot/labwired-core) | 1 | 0 | 0 / 470 | `fcd2f239be30ac59a6c6121bf80475a9a71c1990` |
| [godofecht/labwired-core](https://github.com/godofecht/labwired-core) | 1 | 0 | 0 / 549 | `e5666ead39007f02ca9c806865d02d529eee4bc4` |
| [rifont/labwired-core](https://github.com/rifont/labwired-core) | 1 | 0 | 0 / 611 | `6de763141e0c8ab074011dd4601c1eb5e4ca4a36` |
| [forrepid/labwired-core](https://github.com/forrepid/labwired-core) | 1 | 0 | 0 / 1015 | `3a27b3f843c29fe7c5a802e45178884671812b85` |
| [MJP-Sys/labwired-core](https://github.com/MJP-Sys/labwired-core) | 1 | 0 | 0 / 1422 | `bbcde6748a49af741355673361dacd5522c84f59` |
| [CrispStrobe/labwired-core](https://github.com/CrispStrobe/labwired-core) | 294 | 294 | 190 / 16 | `d543a46bba894f9e7e3aa31cdba6feb561b2d08e` |
| [jdefrancesco/labwired-core](https://github.com/jdefrancesco/labwired-core) | 1 | 0 | 0 / 3090 | `5bf645d731f409a60dc53e33099654254dd45e08` |
| [anikaku12/labwired-core](https://github.com/anikaku12/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [biankafritsch/labwired-core](https://github.com/biankafritsch/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [elnamuller1/labwired-core](https://github.com/elnamuller1/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [smitham24/labwired-core](https://github.com/smitham24/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Madelynn54jhkhf/labwired-core](https://github.com/Madelynn54jhkhf/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Mona5153Skiles/labwired-core](https://github.com/Mona5153Skiles/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [LawsonStoltenberg/labwired-core](https://github.com/LawsonStoltenberg/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [PercivalKlocko/labwired-core](https://github.com/PercivalKlocko/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Anarkali2026/labwired-core](https://github.com/Anarkali2026/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Jody5Krajcik/labwired-core](https://github.com/Jody5Krajcik/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Tianna4Steube/labwired-core](https://github.com/Tianna4Steube/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Aaron41Larkin/labwired-core](https://github.com/Aaron41Larkin/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Greta968Weissnat/labwired-core](https://github.com/Greta968Weissnat/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Odell6565King/labwired-core](https://github.com/Odell6565King/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Rosemarie1f/labwired-core](https://github.com/Rosemarie1f/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Minerva1Stamm/labwired-core](https://github.com/Minerva1Stamm/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Ryan5224Bode/labwired-core](https://github.com/Ryan5224Bode/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Deron521W/labwired-core](https://github.com/Deron521W/labwired-core) | 1 | 0 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
| [Celestino87547/labwired-core](https://github.com/Celestino87547/labwired-core) | 1 | 0 | 0 / 3251 | `81c3e2c0f423fd238bea322ef7fc8ce3e983a277` |
| [lokijhy25/labwired-core](https://github.com/lokijhy25/labwired-core) | 1 | 0 | 0 / 3305 | `7570e7c2ee0946ab2dd36b24d445ec0cb38a9d6c` |
| [Albert368892Leffler/labwired-core](https://github.com/Albert368892Leffler/labwired-core) | 1 | 0 | 0 / 3305 | `7570e7c2ee0946ab2dd36b24d445ec0cb38a9d6c` |
| [Vada2025Bailey/labwired-core](https://github.com/Vada2025Bailey/labwired-core) | 1 | 0 | 0 / 3305 | `7570e7c2ee0946ab2dd36b24d445ec0cb38a9d6c` |
| [Reinhold24Waelchi/labwired-core](https://github.com/Reinhold24Waelchi/labwired-core) | 1 | 0 | 0 / 3305 | `7570e7c2ee0946ab2dd36b24d445ec0cb38a9d6c` |
| [Cleta3217Jacobs/labwired-core](https://github.com/Cleta3217Jacobs/labwired-core) | 1 | 0 | 0 / 3305 | `7570e7c2ee0946ab2dd36b24d445ec0cb38a9d6c` |
| [lotikid36it/labwired-core](https://github.com/lotikid36it/labwired-core) | 1 | 0 | 0 / 3305 | `7570e7c2ee0946ab2dd36b24d445ec0cb38a9d6c` |
| [Monira20/labwired-core](https://github.com/Monira20/labwired-core) | 1 | 0 | 0 / 3305 | `7570e7c2ee0946ab2dd36b24d445ec0cb38a9d6c` |
| [Nikolas689jk/labwired-core](https://github.com/Nikolas689jk/labwired-core) | 1 | 0 | 0 / 3305 | `7570e7c2ee0946ab2dd36b24d445ec0cb38a9d6c` |
| [layer1labs/labwired-core](https://github.com/layer1labs/labwired-core) | 1 | 0 | 0 / 3394 | `867d03cef48ea78c3d9d23d1e3a9e4ea680dc813` |
| [Nectrion/labwired-core](https://github.com/Nectrion/labwired-core) | 1 | 0 | 0 / 3400 | `8d127608b3fa9d049a5884b9bf14f4facc624a49` |
| [MbedLabs/labwired-core](https://github.com/MbedLabs/labwired-core) | 52 | 30 | 0 / 136 | `fecb89651c0819807c52f9889b3c5be75141bfdc` |
| [blue-az/labwired-core](https://github.com/blue-az/labwired-core) | 34 | 25 | 0 / 3224 | `66296c63318644bc3ab8cbb54d79692c38b8e102` |
