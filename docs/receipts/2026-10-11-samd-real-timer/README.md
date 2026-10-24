# Actual Timer caller — original positive qualification

Historical source-head result. Final integration and all eight guest cases
have since landed: [final qualification](../2026-10-12-samd-timer-integration/README.md).
That dated result supersedes prospective PR status below; this original JSON
and its artifact identity remain unchanged.

[Run38169003939](https://github.com/CrispStrobe/labwired-core/actions/runs/38169003939),
attempt1, passed both scheduler profiles at LabWired source
`d5fe6d4c31d885082d8416d6687be50c669f6a07`. Qualification merge
`25d252a93edbcd0e7d8a20b0003212febda79445` has tree
`f657be53ec28b4fe5fa7b7498013b9029914c6b9` and parents
`b4c986f361079f9a6ffb5eb9446f67ee97252733` and that reviewed head.
All seven enabled checks at that head passed. This is not a merged Timer lane:
[PR200](https://github.com/CrispStrobe/labwired-core/pull/200) remains draft
pending the required guest adversaries.

The complete unchanged upstream `Timer.cpp` and actual headers came from CODAL
core `2ed19d64ec3359143409a4b0a9758b4aeef3ee7d`. The authored owned adapters
came from CODAL SAMD `d55214588780d737640536b0a58707cd6d69cfce`.
[PR13](https://github.com/CrispStrobe/codal-samd/pull/13) also remains draft.

## Original artifact and separate audit

The sole [artifact11713702788](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11713702788)
is `actual-timer-caller-guest-results`, 20,095 bytes, ZIP SHA256
`44c5510fea19a52f3c442f0f618f4cca0d2ce2e2f1b3dba77663d6b85baf0f57`.
The [unchanged result](result.json) is 6,724 bytes, SHA256
`95235e6a3eba9a6bcb3f6af5556291e757fd62d4d3ff4eea82f114f51d303a2f`.
Original logs, source/build manifests, ELF/map, dependencies and symbols remain
in the official artifact; publishing this receipt is not another guest run.

A separate standard-library audit, without importing build/result producers,
verified official run/head/attempt/event/success, sole artifact identity/size,
bounded unique ZIP members, actual merge/tree/parents through Git metadata,
source-before/build equality, both original log hashes and complete result rows,
and consumed ELF/map byte hashes. All 21 owned/upstream source files matched
their immutable repository bytes. The three compiler component identities are
recorded, not independently acquired/rehashed; this is not full toolchain
closure, independent-agent testimony, physical qualification or legal clearance.

## Observed boundary

Each profile completed in 45,344 instructions, within the 250,000-instruction
bound. Both observed initialized BSS/copied data, all vectors, six ordered
caller milestones, two actual IRQ0 entries and returns, handler mask0/
exception16 with closed paired state, fault0, and final masked-thread status
`0x600d`. Both consumed the same ELF, SHA256
`61bbd44ef36a34293326c97a68768b4350ed393ad087f2cea08f65171c1c9f0a`.

Actual constructor/sync/calibration, nested setEvent/triggerIn, disabled entry,
cancel/recompute, unmatched cancellation, outer-pair sync, event exhaustion and
destruction ran with explicitly diagnostic clock, allocation and component
adapters. This does not qualify peripheral timers, real fibers, event bus,
allocator, full runtime, USB/WebUSB, native Arcade boot, physical calibration,
licensing or active RTx.

## Next acceptance lane

[PR201](https://github.com/CrispStrobe/labwired-core/pull/201) prepares three
owned-adapter mutants: unconditional restoration, early restoration and explicit
entry into unsupported Event dispatch. Build success alone cannot establish
rejection. Execute each in both scheduler profiles; require exact failure,
fault/mask/exception state and 64 consecutive terminal observations. Timeout,
missing witnesses and unrelated compiler/engine failure are not passing rejection.
Bind original/generated sources and actual compiler/header/object/ELF/log/run
identities, separately audit, and complete every final-head gate before merging
the Timer lane. Preserve this first positive packet unchanged.
