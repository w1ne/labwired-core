# Cortex-A53 GICv2 imported profile

## Measured profile

The imported target is narrowed to one Cortex-A53 processing element and the
2 GiB DDR window at `0x40000000`. The inherited `gicv2` name does not mean this
profile models a GIC: the generic timer, GICv2, PL011 UART, and PL031 RTC from
the imported descriptor are omitted.

## Smoke evidence

The exact imported system ran the committed AArch64 integer-loop fixture for
100,000 instructions. The runner stopped at `max_steps`, passed its one smoke
assertion, and recorded zero peripheral accesses. This establishes firmware
execution on the bounded CPU-and-DDR profile; it does not test GIC, timer,
UART, RTC, multicore behavior, boot firmware, Renode, or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/cortex-a53-gicv2/` contains
the per-target metric, JUnit result, simulator result, snapshot, and run log.
