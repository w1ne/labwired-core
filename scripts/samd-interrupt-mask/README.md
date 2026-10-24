# Authored interrupt-mask ARM guest control

This dedicated hosted qualification pins the unconnected MIT prototype in
[CODAL PR5](https://github.com/CrispStrobe/codal-samd/pull/5) at
`e15482fd6b537ffd208919c31d568c0bea14d47a`. It changes no production CPU,
peripheral, firmware admission or app dependency. Guest execution is pending.

The builder compiles two owned Cortex-M4 fixtures without standard libraries:
the reviewed header, and a temporary mutant that always restores PRIMASK to
zero. Both must link without unresolved symbols. The positive fixture checks
disabled entry and nested enabled-entry restoration; the mutant must reach
assertion3 (incorrect already-disabled restoration), not fault, time out or
fail compilation. Each test observes vectors, poisoned-BSS clearance, entry100
and the exact terminal status within100000 instructions, in both default and
event-scheduler profiles. Compilation alone is not execution evidence.

The receipt binds the actual PR merge commit/tree/parents, reviewed head,
fixture pin, run/attempt, logs and observed ELF hashes. Artifacts contain only
receipt, result logs, maps and symbols; no firmware binary is uploaded. An
independent receipt/log/source audit is still required before publishing a
pass. This initial workflow always executes; it does not claim docs-only reuse.
Do not push receipt-only prose while qualification is pending or repeat an
already successful run unnecessarily. A future reuse gate must bind all
engine/config/dependency/test/build/workflow inputs, not just this fixture pin.

This bounded control is not pending/active IRQ delivery, NMI/HardFault masking,
NVIC vector handling, DMA memory ordering, fiber/driver integration, hardware
behaviour, ASF removal or runtime-image admission. No new performance or
whole-component licence claim follows. A complete vector/NVIC positive and
negative fixture is the next separate qualification before atomic integration.

## First execution: results passed, receipt failed

[Run37904852581](https://github.com/CrispStrobe/labwired-core/actions/runs/37904852581)
at reviewed source `452e6b76ef6cc884e4265bd9c1e26444388f5056` observed
positive `0x600d` in315 instructions and mutant assertion3 in126, in both
profiles. Its overall conclusion is **failure**: the receipt's parent query
used revision traversal, which hides parents at the shallow checkout boundary.
The original logs and six-member artifact preserve these results; no successful
qualification receipt exists for that run. The actual merge object's tree
matched the reviewed source and its second parent matched that source head.

The corrected recorder reads the actual commit object's headers, retaining
the two-parent and exact reviewed-head requirements without history fetching.
Synthetic controls cover that shallow boundary, ignore parent-like commit
message text, and reject wrong/malformed parents, fixture, status, missing
results and ELF hashes. They are production-script controls, not independent
guest evidence. The changed recorder/workflow requires fresh hosted validation.
