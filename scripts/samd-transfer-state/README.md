# Owned ARM state-control qualification — pending

Source fixture: [CODAL-SAMD PR3](https://github.com/CrispStrobe/codal-samd/pull/3),
exact commit `592748e51e6b82238d9c1d6a69ffa4e74ccc4ce1`.
This is an authored abstract transfer-state test, not a CODAL runtime image,
real SPI/DMA transfer, callback/fiber, IRQ or native PyBadge qualification.
The [replacement plan](../../docs/engineering/samd-runtime-replacement-plan.md)
still requires those separate lanes and complete header/build provenance.

The dedicated hosted workflow builds both exact source and a temporary
premature-success header mutation without standard libraries. The normal
guest must reach `0x600d`; the mutant must reach precisely assertion marker 4,
not an arbitrary fault, timeout or host assertion. Resolve `control_status`
from each ELF, poison its BSS witness, and observe guest startup clearing it
and entering the probe before accepting either result. Both engine feature
profiles must run the ignored test explicitly. A normal workspace test skip
is not qualification. Source/image/map hashes and result markers are retained;
ELFs and objects are not uploaded by this workflow. No performance is measured.

The fixture disables interrupts and uses diagnostic startup/memory layout.
Its DMA/TXC calls are authored abstract events, not real peripheral observations.
The first actual hosted guest result and every enabled exact-head PR check
are required before landing this test harness. Preserve any failed result;
never change the expected marker or invent completion to make it pass.

## Documentation-only source-bound reuse

The first actual [run37889228375](https://github.com/CrispStrobe/labwired-core/actions/runs/37889228375)
passed both profiles: positive `0x600d`/1425 instructions, premature-success
mutation assertion 4/302 instructions. It predates the provenance record below,
so the new gate cannot automatically reuse it. The first run of the new workflow
must qualify its changed harness once and retain `qualification.json`.

Later runs may reuse an official successful fresh execution only when its run,
attempt, merge/head/tree, fixture pin, artifact digest and exact results in both
profiles bind correctly. Compare the entire Git snapshot: only this README
and `docs/testing/IGNORED_TESTS.md` may differ. Engine, dependency, config, test,
build script, workflow or reuse-policy changes require fresh execution.
Cancelled/failed runs and artifacts recording only reuse cannot supply fresh
qualification. All unrelated enabled final-head checks remain required.
The reused result is evidence about the original run, not new guest execution.

### Fresh provenance-bound result — remaining PR gates pending

[Run37900687425](https://github.com/CrispStrobe/labwired-core/actions/runs/37900687425),
attempt 1, source `15672efdb3b547b6173e8458ea0707626c8c38d7`, passed every
guest-job step. Its actual checkout `670c7281803d13aeb573a7eb5da41996d74291aa`
has the reviewed tree `e2524d22211d9fdee39a54cfaedd32f7ac10f37c`.
Both profiles again observed BSS clearing/probe entry and exact positive
`0x600d`/1425 instructions, mutant assertion 4/302 instructions.
Positive ELF SHA256 remains
`bd539fda7f60b37b82e9e47b96180dff2208718d9e1da8057c5a8a82408ab2d3`;
mutant is `f2191da7b4214aff866aad4c863b476483f720ff9b1550f1c54c5103a17a3949`.

[Artifact11602762320](https://github.com/CrispStrobe/labwired-core/actions/runs/37900687425/artifacts/11602762320)
is 5505 bytes, ZIP SHA256
`2e8a046955140fec36b4687bda1fae25fa579b1d0dbd0ad59f0a58b28077768d`.
A separate raw ZIP/metadata/hash audit verified the fresh decision, exact
provenance, both result pairs, successful steps and safe eight-member inventory
without ELF/object uploads. This is not independent ELF or hardware replay.
The documentation-only publication of this record should exercise reuse of
that fresh qualification. Its reuse check and all other enabled final-head
checks are still required; this paragraph does not claim merge readiness.
