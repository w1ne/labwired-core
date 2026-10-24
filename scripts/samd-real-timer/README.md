# Actual Timer guest qualification harness

This is a separate actual-guest qualification lane for [CODAL PR13](https://github.com/CrispStrobe/codal-samd/pull/13),
not another run of the already qualified five authored paired-policy cases.
Hosted [control run](https://github.com/CrispStrobe/labwired-core/actions/runs/38163162860)
compiled the harness and passed three manufactured Rust groups and five Python
reader groups. The [source ARM build](https://github.com/CrispStrobe/codal-samd/actions/runs/38163094316)
passed at `d55214588780d737640536b0a58707cd6d69cfce`. The dedicated
`samd-real-timer-guest.yml` workflow now selects that exact source for both
scheduler profiles. The [first positive run](https://github.com/CrispStrobe/labwired-core/actions/runs/38169003939)
passed both profiles, with a separate original-packet/source audit. See its
[unchanged receipt](../../docs/receipts/2026-10-11-samd-real-timer/README.md).

The ignored test requires `LABWIRED_TIMER_FIXTURE_DIR` and an explicitly reviewed
`LABWIRED_TIMER_QUALIFIED_SOURCE`. That source parameter verifies identity, not
GitHub qualification: require every enabled source build/control before selecting
it. The builder's source/core/ELF provenance must match the consumed image.

The harness poisons full diagnostic BSS and data RAM, then checks their reset
initialization at the actual caller entry, including copied executable data.
It validates all vector words, nine distinct BSS witnesses, each of six ordered
caller milestones, exactly two actual IRQ0 entries and returns, closed paired state at IRQ
delivery, exception16/mask0 in the handler, thread return, fault0 and masked
terminal `0x600d`. The bound is 250,000 instructions; timeout is never success.
Three manufactured witness controls are not actual guest qualification.

`record.py` is a prospective receipt producer, not an independent audit. It
requires both complete execution logs, the consumed ELF/map, the before/after
build manifest, exact source/header/toolchain inventory and live immutable Git
source bytes. It refuses source drift, unbound headers, missing/duplicate results,
wrong source/image, missing witnesses and out-of-bound execution. The CLI binds
the actual qualification merge/head/tree/parents and official run/attempt, and
refuses to overwrite a previous receipt. `test_record.py` has five manufactured
groups with seventeen validation refusals and nine entry-point refusals. The
entry-point controls also check exclusive receipt creation and no writes after
rejected source/run/merge context; these tests contain no actual guest result.

The [three actual adapter adversaries](../../docs/receipts/2026-10-12-samd-real-timer-refusals/README.md)
also passed both profiles with a separate source/object/log audit. The complete
lane now includes current-main documentation and the stacked negative harness.
Keep the PR draft until every affected final integration-head check and actual
guest passes; earlier source-head results are not final integration results.
Do not move existing consumer pins or overwrite earlier original packets.

Clock, allocation, component registration and event dispatch remain explicitly
limited by the [runtime fixture contract](https://github.com/CrispStrobe/codal-samd/blob/d552145/timer-caller/README.md).
This does not qualify peripheral timers, real fibers, event bus, full runtime,
native Arcade boot, physical calibration, licensing or active RTx.
