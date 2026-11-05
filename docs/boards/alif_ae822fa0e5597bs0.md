# Alif Ensemble E8 imported profile

## Measured profile

The imported Alif E8 descriptor is narrowed to one Cortex-M55-HP core with
256 KiB of local ITCM at `0x00000000` and 1 MiB of local DTCM at
`0x20000000`. The descriptor's `flash` slot serves as the executable ITCM load
window; it does not model the device's MRAM or boot copy process. Alif documents
the E8 device as having Cortex-M55 cores and these local memory windows
([E8 series](https://alifsemi.com/ensemble-e8-series/),
[memory map](https://swrm.alifsemi.com/Content/2.6%20Shared%20Memory.htm?TocPath=Overview+of+the+Device+System+Architecture%7C_____5)).

The profile omits the second Cortex-M55, both Cortex-A32 cores, Helium/MVE,
peripherals, and boot firmware. These are not covered by this smoke.

## Smoke evidence

The exact imported system ran the committed Thumb integer-loop fixture to the
100,000-step `max_steps` limit and recorded zero peripheral accesses. The log
records 48 interpreted instructions and 99,952 idle-fast-forwarded cycles. The
fixture uses common integer/Thumb instructions assembled for a Cortex-M33-
compatible subset. This establishes execution of that subset on the bounded
CPU-and-TCM profile. It does not validate the full Cortex-M55 ISA, Helium,
peripherals, multicore behavior, boot firmware, Renode, or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/alif_ae822fa0e5597bs0/`
contains the per-target metric, JUnit result, simulator result, snapshot, and
run log.
