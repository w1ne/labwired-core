# Priority-order probe

This target-only run deliberately reversed the deferred-IRQ priorities to probe
Cortex-M0 preemption. It booted and executed 4,000,000 instructions, but the
existing smoke correctly failed its deferred-pending assertion: IRQ 6 had the
higher priority (`0x40`) than IRQ 5 (`0x80`), so it was taken before IRQ 5
resumed. The run is preserved as diagnostic evidence; the committed smoke uses
IRQ 5=`0x40` and IRQ 6=`0x80` to test deferral and tail-chaining.
