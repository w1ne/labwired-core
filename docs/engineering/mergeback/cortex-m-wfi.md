# Prepared Cortex-M sleep mergeback

This patch is a review artifact. It is **not applied to the active CPU source by this PR**. The PWM/I2C mergeback and this CPU candidate have different validation gates.

## Source and exact patch

Base: `3e227a9b3b8332b38d33a721d2662b237a185ebb`.

Patch: [cortex-m-wfi.patch](cortex-m-wfi.patch).

SHA-256: `9f82d16a45a545613ba2ff2a0c09fbb65fc26e0437e62bd5b130613bafad6ce8`.

The original behavior comes from [CrispStrobe c731d22a0](https://github.com/CrispStrobe/labwired-core/commit/c731d22a0a7bccadec0916220d277c80ae1f95ab), with test ideas from [765e3ae79](https://github.com/CrispStrobe/labwired-core/commit/765e3ae79f9f32b5ad91693eb2938dbacb5260fb) and [e5d61d3d7](https://github.com/CrispStrobe/labwired-core/commit/e5d61d3d770ae094f0a53dc4c56e81c4f33070c1). The port adds compiled-runner guards and wake reconciliation found necessary during the upstream review. The source license remains MIT.

## Behavior

- A WFI core remains parked while continuous peripheral work prevents idle time skipping.
- Shared enabled NVIC pending lines prevent a sleep skip before the normal CPU import boundary.
- Already-imported but subsequently cleared or disabled external IRQs do not falsely wake WFI.
- Both compiled runners route sleeping execution through the CPU's sleep checks.
- On an actual WFI/WFE resume, a cold helper reconciles external pending bits with live `ISPR & ISER` before the existing exception dispatcher. CPU-owned exceptions 0–15 remain intact.
- Existing idle-disabled WFI behavior is preserved. The exception dispatcher and SAMD execution chunk machinery are unchanged.

The three changed files are `crates/core/src/cpu/cortex_m.rs`, `crates/core/tests/cortex_m_event_sleep.rs`, and `crates/wasm/src/jit_browser.rs`.

## Measured verification on 2026-11-10

| Run | Result |
| --- | --- |
| Original upstream plus initial WFI regression tests | 8 passed, 3 failed: shared wake admission, cleared/disabled shared wake admission, and post-WFI execution during peripheral service. |
| Corrected v3, default event-sleep suite | 13 passed, 0 failed. |
| Native JIT + event scheduler, deliberate old-guard/stale-pending negative control | 12 passed, 2 failed: warmed compiled cache and cleared/disabled imported pending bits. |
| Native JIT + event scheduler, corrected v3 | 14 passed, 0 failed. |

The native test first proves that the post-WFI loop executes compiled code. The negative control then advances its counter from 7,802 to 7,922 while asleep. With the corrected source, the counter and compiled-block count remain unchanged while asleep. The same test covers idle-disabled behavior and resumed compiled execution after a PRIMASK-masked wake.

Other cases cover 28 wake-eligibility combinations, eight direct wake/dispatch combinations, live IRQ masks, IRQ0/IRQ37, and 37 peripheral service cycles with batch caps 1 and 17. These are assertions within the test suites, not additional independent suite counts.

Both positive profiles used Rust 1.95.0, optimization level 0, debug information disabled, one build job, and incremental compilation disabled. The exact test invocations were:

```sh
cargo +1.95.0 test -p labwired-core --test cortex_m_event_sleep -- --nocapture
cargo +1.95.0 test -p labwired-core --features 'jit event-scheduler' --test cortex_m_event_sleep -- --nocapture
```

The profile overrides were `CARGO_PROFILE_DEV_OPT_LEVEL=0`, `CARGO_PROFILE_TEST_OPT_LEVEL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_BUILD_JOBS=1`, and `CARGO_INCREMENTAL=0`. The tests used a separate build directory.

Rustfmt, whitespace checks, clean patch application against the pinned base and exact positive-source restoration after the negative control all passed. The published patch uses zero context so the patch artifact itself has no trailing-whitespace lines. Applying it and the tested full-context v3 patch to separate indexes at the pinned base produced the same Git tree: `25f2f78b923b15d8dd3e239df34bbcc84db70e75`. Independent source review found no blocking issue.

## Gates before an active-source PR

The patch changes `cortex_m.rs`, watched by seven silicon-capture records: `nrf52840`, `seeed-xiao-nrf52840-sense`, `stm32h563`, `nucleo-l476rg`, `nucleo-l073rz`, `stm32f103`, and `stm32f407`. `validation/manifest.yaml` reports source/capture differences without a mandatory human source-review acknowledgement. Existing acknowledgements are historical metadata and cannot establish a source match. These software tests do not renew a hardware capture; a missing captured digest remains unverified.

Browser runtime execution, full CPU suites, debug-step behavior during WFI, and performance measurements remain unverified. Test those relevant modes before promotion. Keep the upstream performance baselines and historical capture and acknowledgement values unchanged. The normal automated validation and performance gates still apply.

Apply only after reviewing the pinned base and validation gates. The zero-context artifact needs `--unidiff-zero`. To check application without changing the working tree:

```sh
git apply --unidiff-zero --check docs/engineering/mergeback/cortex-m-wfi.patch
```
