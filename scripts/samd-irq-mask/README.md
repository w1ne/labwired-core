# Authored software-pended IRQ0 control

Dedicated hosted qualification of [CODAL PR6](https://github.com/CrispStrobe/codal-samd/pull/6),
source `2a991760339fe89f5272f923baf89dc18fd434d0`. No production engine,
peripheral, driver, app pin or runtime admission changes. Execution is pending.

The positive fixture must deliver exactly one IRQ0 after the outer mask is
restored. The temporary restore-always-enabled mutant must deliver it early:
exact assertion6 with count1/stage1/mask0/IPSR16, not just a changed mask.
Positive requires600d/count1/stage2/mask0/IPSR16. Both profiles require actual
CPU exception16 entry and return to thread, full256-word vector validation
against ELF symbols/reserved zeros, poisoned-BSS clearance and probe entry.
Only BSS witnesses are poisoned; the host does not inject or clear pending IRQs.
Fault, timeout, compile failure, alternate assertion and trapf00d cannot pass.

Builds are source-bound and library-free. Fresh receipt binds actual checkout
commit/tree/raw-header parents, reviewed head, fixture pin, run/attempt, logs
and observed ELF hashes. Artifacts contain receipt/logs/maps/symbols, not ELF.
Separate original metadata/artifact/source audit remains required before a
qualification claim. This workflow always executes; no reuse claim. Keep the
source frozen while the first run completes and preserve any original failure.

This is software-pended IRQ0, not SAMD device/DMA signalling, preemption,
priorities, level acknowledgement, fibers, hardware timing or complete runtime
startup. Source access/origin and architectural limits are recorded in the
[fixture contract](https://github.com/CrispStrobe/codal-samd/blob/test/simulation-irq-mask-delivery-20261009/simulation/IRQ-MASK-CONTROL.md).
No clean-room, whole-image licensing, app adoption or RTx claim follows.
