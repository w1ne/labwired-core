# Actual Timer caller — landed integration qualification

[PR200](https://github.com/CrispStrobe/labwired-core/pull/200) merged as
`f45d0069c042456333ee472a90bf1d1e9e94c1e6` after **all 28 enabled checks passed**,
including both aggregates and three workspace shards. Four declared full/warm/image
jobs skipped. Tested source was `126ce5394e8be26d078fa333f7d6965a78e8a2bb`.
[Rust CI38216803616](https://github.com/CrispStrobe/labwired-core/actions/runs/38216803616)
passed. Reviewed, actual guest, predicted merge and landed trees equal
`1d76d37ef8c2c536c9be94b23fb6cb564803dd7b`; merge parents are refreshed main
`ce5091d8d9bf5c64f625cafad928af2f418c51d2` and the tested source.
The redundant stacked [PR203](https://github.com/CrispStrobe/labwired-core/pull/203)
was closed as superseded, not independently merged.

## Actual guests and unchanged original receipts

Both runs used attempt 1 and qualification merge
`44bc038b75fa58374b0a5a907f7085380dc433b2` with the same tree and parents above.
The complete unchanged upstream Timer came from CODAL core
`2ed19d64ec3359143409a4b0a9758b4aeef3ee7d`; opt-in diagnostic adapters came from
CODAL SAMD `d55214588780d737640536b0a58707cd6d69cfce`, landed through
[CODAL PR13](https://github.com/CrispStrobe/codal-samd/pull/13).

| Qualification | Original run | Profiles | Result |
| --- | --- | --- | --- |
| Positive caller | [38216803677](https://github.com/CrispStrobe/labwired-core/actions/runs/38216803677) | Default and scheduler | Each 45,344 instructions; six ordered milestones, two actual IRQ entries and returns, fault 0, final masked thread status `0x600d` |
| Adapter refusals | [38216803650](https://github.com/CrispStrobe/labwired-core/actions/runs/38216803650) | Three cases in each profile | Exact unconditional `0xb02` / early `0xa14` / unsupported Event `0xa07`; each 64 terminal instructions |

Both positive profiles checked vectors, poisoned BSS/data initialization and
handler mask 0 / exception 16 / closed paired state. The refusal cases took
24,384 / 26,301 / 1,331 instructions respectively in each profile; all had
fault 0 and mask 1. Early restoration observed exception 16, one actual IRQ
entry and one milestone; the other cases remained in thread context with none.
Wrong failure, escaped or stopped terminal PC, success, engine fault and timeout
refuse. Finite terminal execution is not indefinite liveness or hardware safety.

| Original | Bytes | SHA256 |
| --- | --- | --- |
| [Positive artifact11733711959](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11733711959) ZIP | 20,060 | `e6818f47f744d80d68d22eb670c83405290332d81f99f98f4b38b84e6f145336` |
| [Positive raw receipt](positive-result.json) | 6,724 | `5042ec6296cf37f66588336eba64ab2b6812d5bc941eca2c0eb6b7e68547189f` |
| [Refusal artifact11732645920](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11732645920) ZIP | 80,162 | `831d8b91b444fa71f0395e811045a9aeda3dde6ed5c607569e1ac5753b30f8d7` |
| [Refusal raw receipt](refusal-result.json) | 14,674 | `37282b707b37741f7a0c1c969a588c2c67a11bb11a26d763e60fc90bc94b23c3` |

Each sole official artifact was acquired once. Separate standard-library audits
without producer imports or guest replay verified official run/head/attempt,
bounded unique ZIP members, actual merge metadata, all 21 original Git source
files, build/source equality, consumed ELF/map bytes and complete logs/results.
The negative audit additionally checked three helpers, exact source derivatives,
changed-unit dependency closures and all consumed object bytes. Recorded compiler
component identities are not independently acquired binary or full-toolchain
closure. No independent-agent implementation testimony is claimed.

## Preserved failures and remaining scope

[Run38205449026](https://github.com/CrispStrobe/labwired-core/actions/runs/38205449026)
failed Clippy on a redundant refusal-harness conversion;
[run38208063921](https://github.com/CrispStrobe/labwired-core/actions/runs/38208063921)
then found the same conversion in the positive harness. Both failures remain
recorded; neither was waived. The final source removed both conversions and
passed Clippy without changing assertions, thresholds or the lint policy.
Earlier [positive](../2026-10-11-samd-real-timer/README.md) and
[refusal](../2026-10-12-samd-real-timer-refusals/README.md) originals remain unchanged
historical source-head results, not final integration evidence.

This qualifies the actual Timer caller with explicitly diagnostic clock,
allocation, component and event adapters, not a whole application runtime.
No app dependency pin, default CODAL build, hardware capture or drift
acknowledgment changed. Actual clock/component/allocator/event/fiber services,
required USB/WebUSB, named no-ASF build and component obligations remain open.
Next is the separately qualified [SERCOM IRQ slice](../../engineering/samd-runtime-replacement-plan.md#next-bounded-foundation-and-integration-tasks),
then DMA and asynchronous CODAL completion. Native Arcade boot, installed
debugger/package adoption, physical calibration and active native/WASM RTx remain
unqualified. This publication is documentation only, not another execution.
