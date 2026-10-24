# Authored strict paired-IRQ ARM guests — original qualification

## Corrected final-head guest gate

The generated ignored-test inventory required a correction after the first guest
pass. [Run38158310794](https://github.com/CrispStrobe/labwired-core/actions/runs/38158310794),
attempt1, then passed all ten cases at final head
`b4c986f361079f9a6ffb5eb9446f67ee97252733`. All recorded ELF identities and
case results match the original pass below; source/policy/bounds are unchanged.
This was automatic CI after a required gate fix, not a diagnostic replay for
reading evidence. The original failed default-member index gate remains in
[run38153786840](https://github.com/CrispStrobe/labwired-core/actions/runs/38153786840).

The sole [artifact11709799464](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11709799464)
is `authored-paired-irq-guest-results`, 16,642 bytes, ZIP SHA256
`4381f1d0ac7805eb99ed7add47016fac4cbabac16bf5ae55bce1c5b80cd687ff`.
The [unchanged final-head result](final-head-result.json) is 4,291 bytes, SHA256
`9a319216f6546080802e3281a23c19105e7661e80186769ce2a73014b28d4117`.
The same separate no-producer-import packet/source audit passed on its official
identity, six source files, snapshots and both execution logs. Official Git
metadata confirms qualification merge `0621e95366b2f55a1369f373f96c3ec639eaa956`,
tree `539beada886c0fd51425842ac03061a65296e638`, parents
`78ec4af563b70466d8e33c79193c0e270d513230` and final head `b4c986f...`.
All 20 enabled final-head checks passed, including both aggregate gates, with
only the four declared optional jobs skipped. [PR198](https://github.com/CrispStrobe/labwired-core/pull/198)
merged as `46aed9ba8232b3cceaec8a33e15230f9d973d486`; the predicted, reviewed,
actual-guest and landed trees all equal `539beada886c0fd51425842ac03061a65296e638`.
Landed parents are the same base and final head listed above. This remains a
bounded authored-control result. Both original packets/results are preserved.

## Earlier unchanged original pass

[Run38153786907](https://github.com/CrispStrobe/labwired-core/actions/runs/38153786907),
attempt1, passed at head `e7589656998d3a08cd59670ff09a4f3214310d2e`.
Qualification merge `f074b54d48cf4381b6832aab7ce5abc4aaea45d3` has tree
`3c2c12a69ed4abdab301b5528eeb5cd8d462e6a6` and parents
`78ec4af563b70466d8e33c79193c0e270d513230` and that tested head.
The earlier pass alone did not satisfy the broader merge gate; that gate was
subsequently completed at the corrected final head described above.

The guest consumed CODAL source `3c66f0d27c865192a2b22217e71c1f88f2b3fe91`,
after all eight enabled source checks passed. [CODAL PR10](https://github.com/CrispStrobe/codal-samd/pull/10)
merged as `c2274c7ed397787820c92a91438c4c0a902612b8`, with reviewed/landed tree
`8ed57db57606e4ea644609e33c6eaf385beb623b` equal. That merge did not select
these opt-in components in the default runtime CMake build.

## Original artifact and separate audit

The sole [official artifact11707374717](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11707374717)
is `authored-paired-irq-guest-results`, a 16,643-byte ZIP, SHA256
`374e6bd22a8c7eec1de729a9e14e63f837e79bb5d4309f723b360aac2570a83f`.
The [unchanged original result](result.json) is 4,291 bytes, SHA256
`ff7a67aabfcf7410da46020fc2b067f914d8808a6d05e9a915b663b62b444d4e`.
The original contains both logs, source snapshots, maps and symbol reports;
ELFs are deliberately not bundled. This durable result is not a new execution.

A separate standard-library audit without importing the producer checked
official run/head/attempt/success and artifact identity/size, bounded unique ZIP
members, qualification merge/tree/parents, equal before/after snapshots, six
actual CODAL source hashes, both log hashes, all ten result rows and identical
recorded ELF identities across profiles. It passed. Official Git commit metadata
also confirmed the qualification merge's tree and parents. This is packet/source
audit, not replay, independent-agent testimony, full compiler-component closure
or independent ELF rehashing from absent binaries.

## Ten observed cases

Both default and event-scheduler profiles produced the same following results:

| Authored case | Status | Fault | Guest steps, each profile | Terminal refusal instructions |
| --- | --- | --- | --- | --- |
| Positive nested/restoration/IRQ | `0x600d` | 0 | 3791 | 0 |
| Underflow | `0x201` | 3 | 369 | 64 |
| Changed diagnostic identity | `0x202` | 5 | 624 | 64 |
| Depth limit | `0x203` | 4 | 16644 | 64 |
| Lost masking | `0x204` | 6 | 638 | 64 |

The [tested harness](https://github.com/CrispStrobe/labwired-core/blob/e7589656998d3a08cd59670ff09a4f3214310d2e/crates/core/tests/samd_paired_irq_guest.rs)
checks vectors, startup BSS/data initialization, mask restoration, one positive
stage2/mask0/exception16 IRQ and thread return. Refusals require unchanged exact
diagnostics, masked thread mode and zero IRQ witnesses. Only the actual compiled
mask-restore helper is allowed during transition, for at most 32 instructions;
64 consecutive terminal observations reset across it, with a 192-observation
transition bound. Caller escape, unrelated code, stuck helper and stopped PC
refuse. Four pure progress controls ran before the actual guest cases.

This is bounded liveness, not indefinite behavior. The diagnostic identity is
not real fiber switching. Additional guest-level policy mutants and full
recording-entry-point adversaries remain open. Real Timer/Fiber/Heap and
startup/panic/scheduler integration, compatible core/board inputs, a named
no-ASF build, required USB/WebUSB, component notices, peripheral IRQ/DMA,
native Arcade boot, debugger/package adoption and active RTx remain unqualified.
Continue through the [interface contract](../../engineering/samd-critical-section-interface.md)
and [replacement lanes](../../engineering/samd-permissive-runtime-lanes.md).
