# Cortex-A53 GICv3 imported profile

## Measured profile

The imported target is narrowed to one Cortex-A53 processing element and the
2 GiB DDR window at `0x40000000`. The inherited `gicv3` name does not mean this
profile models a GIC: the generic timer, GICv3, PL011 UART, and PL031 RTC from
the imported descriptor are omitted.

## Smoke evidence

The exact imported system runs the committed AArch64 integer-loop ELF from
`0x40080000` for 100,000 instructions. The onboarding runner records the
`max_steps` stop, passes its one smoke assertion, and observes zero peripheral
accesses. The result records an ideal-rail assumption, so power-on and
brown-out behavior are not tested. This establishes CPU execution on the
bounded core-and-DDR profile; it does not test GICv3, timer, UART, RTC,
multicore behavior, reset-vector boot, Renode, or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/cortex-a53-gicv3/` contains
the per-target metric, JUnit result, simulator result, snapshot, and run log.
