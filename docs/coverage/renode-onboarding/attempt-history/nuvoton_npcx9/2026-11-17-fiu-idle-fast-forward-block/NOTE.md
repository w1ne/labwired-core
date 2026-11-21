# FIU smoke attempt 01 — idle fast-forward blocked

The first imported-system run with the FIU window and firmware check booted,
and the new FIU checks passed. It failed earlier MTC / TWD timing assertions
and stopped at `max_steps` instead of `max_cycles`.

Cause: the new pure-MMIO FIU model inherited `legacy_tick_active() == true`.
That made it look like active legacy tick work and disabled the machine's WFI
idle fast-forward, so the 2,000,000 instruction cap expired during the watchdog
wait. The model has no periodic work; the follow-up overrides both
`legacy_tick_active` and `needs_legacy_walk` to false.

This was a model scheduling integration failure, not a passing measurement.
The attempt is retained here; the target's scoreboard row is replaced only by
the subsequent measured run.
