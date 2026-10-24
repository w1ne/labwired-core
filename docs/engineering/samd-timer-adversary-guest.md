# Timer adapter negative guests — qualified diagnostic harness

The actual six-case qualification and final integration have now landed through
[PR200](https://github.com/CrispStrobe/labwired-core/pull/200). Read the
[final source-bound originals, audits and limitations](../receipts/2026-10-12-samd-timer-integration/README.md).
The prospective preparation below is historical context, not the latest status.

The separate `samd_real_timer_adversary_guest` integration test consumes the
three derivative images from [PR201](https://github.com/CrispStrobe/labwired-core/pull/201).
[Hosted harness controls](https://github.com/CrispStrobe/labwired-core/actions/runs/38172838576)
compiled the harness and passed its three manufactured Rust groups.
[Owned mutant build](https://github.com/CrispStrobe/labwired-core/actions/runs/38172360799)
compiled and linked all three cases. The dedicated
`samd-real-timer-adversary-guest.yml` workflow now enables actual execution in
both scheduler profiles. Workflow preparation is not a negative guest pass.

The actual test requires `LABWIRED_TIMER_MUTANT_DIR`, matching immutable adapter
source, each declared case and the consumed ELF byte identity. It validates all
vectors, nine distinct aligned BSS witness locations, complete poisoned BSS/data
reset initialization and a bounded actual rejection function extent. It checks
every terminal observation against these fixed expectations, not against a
producer's freely selectable expected status:

| Case | Status | Fault | Mask | Exception | Actual IRQ entries | Caller milestones |
| --- | --- | --- | --- | --- | --- | --- |
| Unconditional restore | `0xb02` | 0 | 1 | 0 | 0 | 0 |
| Early restore | `0xa14` | 0 | 1 | 16 | 1 | 1 |
| Unsupported event | `0xa07` | 0 | 1 | 0 | 0 | 0 |

All nine guest witnesses must equal the exact case-specific tuple, including
component/compare counts and recorded handler mask/context. Wrong failure,
success status, engine fault or timeout refuses. The CPU must execute 64
consecutive instructions within the actual compiled `reject` function, preserve
all terminal witnesses and show changing PC, within 250,000 total instructions.
This is bounded terminal behavior, not indefinite liveness or hardware safety.

The receipt producer separately reconstructs each fixed source derivative from
live immutable Git bytes, checks the complete positive source/header inventory,
changed-unit closure, all consumed object/image hashes, both complete logs and
fixed statuses/context/counts. It binds actual merge/head/tree/parents,
official run/attempt and live builder/reader/harness source bytes, and refuses
to overwrite an existing receipt. The recorded driver/libgcc/header component
bytes must also match after negative builds and execution; this limited snapshot
does not establish whole-toolchain closure. Seven manufactured Python groups
include 31 validation refusals, four compiler-component refusals and ten
entry-point refusals; these are not guest evidence.

Completed: dedicated actual execution in both scheduler profiles, original
receipts and separate audits, and all 28 enabled final-head checks before merge.
Existing earlier positive Timer and authored paired-policy receipts remain
unchanged. Real runtime services and peripheral IRQ/DMA integration remain
separate tasks. This lane makes no whole-runtime, licensing, physical
qualification, native-boot or RTx claim.
