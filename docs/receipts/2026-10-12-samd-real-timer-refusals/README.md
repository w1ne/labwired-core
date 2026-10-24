# Actual Timer adapter refusals — original qualification

Historical source-head result. [Final integration qualification](../2026-10-12-samd-timer-integration/README.md)
now records PR200's landed source, final guests and all enabled checks. It
supersedes the pending integration status below, without changing this original
JSON or its artifact identity.

[Run38198468505](https://github.com/CrispStrobe/labwired-core/actions/runs/38198468505),
attempt1, passed all three cases in both scheduler profiles at
`6947637378841a3fe0e249620b8ec8316efe93a3`. All eight enabled checks at that
head passed. Qualification merge `db2863bae5054bad3e23ab32d3ef9d7b9e15102f`
has tree `63158ec109705218ab85d99e2de29bf3dbbf34dc` and parents
`c8f1865a625bd6471197c5e02a59c994c976e48f` and the tested head.

| Authored mutant | Exact status | Fault | Mask | Exception | Actual IRQ entries | Steps per profile | Terminal instructions |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Unconditional restore | `0xb02` | 0 | 1 | 0 | 0 | 24384 | 64 |
| Early restore | `0xa14` | 0 | 1 | 16 | 1 | 26301 | 64 |
| Unsupported event entry | `0xa07` | 0 | 1 | 0 | 0 | 1331 | 64 |

The harness also checked vectors, full BSS/data startup, all nine fixed
case-specific witnesses, instruction execution within the compiled rejection
function and changing PC on every terminal instruction. Wrong status, success,
regression, escaped/stopped PC, wrong mask/context, engine fault or timeout
refused qualification. These are finite observations, not indefinite liveness.

## Original artifact and separate audit

The sole [artifact11724842558](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11724842558)
is `actual-timer-owned-adapter-refusals`, 80,126 bytes, ZIP SHA256
`fd9fba32272de5c4ef535b9d1a96cd9ad7c064b4139f090aeae3aed7d9c13735`.
The [unchanged original result](result.json) is 14,674 bytes, SHA256
`9e82661d36969207df1631a67dc127374e88a84795d7d6ce140ade0e4b3043de`.
Publishing the original receipt is not another execution.

A separate standard-library audit without producer imports checked official
run/head/attempt/event/success, sole artifact identity/size, bounded unique ZIP,
actual merge/tree/parents, source/build equality, 21 original source files and
three qualification helpers against immutable Git bytes, all three exact
generated derivatives, changed-unit dependency closures, consumed object and
ELF/map byte hashes, both log hashes and all six complete result rows. It passed.
Recorded driver/libgcc/header before/after identities matched; their binaries
are not included, so this is not independent compiler-byte or full-toolchain
closure. No guest replay or independent-agent testimony is claimed.

## Integration and remaining boundary

The [first positive caller receipt](../2026-10-11-samd-real-timer/README.md)
and this original packet qualify the unchanged upstream Timer with explicitly
diagnostic clock/allocation/component adapters and authored software IRQs.
[CODAL PR13](https://github.com/CrispStrobe/codal-samd/pull/13) landed the opt-in
fixture after its build, positive guests and these negative guests passed.
No default application configuration or dependency pin was adopted.

The stacked LabWired code is being assembled in
[PR200](https://github.com/CrispStrobe/labwired-core/pull/200) against current
main. Its final integration head still needs its own affected guest and all
enabled checks before merge; this earlier success is not that final-head result.
Actual component registry, allocator, event bus, hardware timer/fiber/startup
integration, required USB/WebUSB, named no-ASF build, native Arcade boot,
component licensing, physical qualification and active RTx remain separate work.
