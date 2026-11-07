# PolarFire SoC initial profile attempt — 2026-11-05

The first exact imported-system run booted and executed 2,000,000 instructions,
but failed both smoke assertions: Zephyr emitted no UART bytes and the runner
stopped at `max_steps` rather than `max_cycles`. The simulator snapshot records
`clint.base: null`; the CPU mapping in `Rv64Params::for_chip_name` selected the
MPFS U54 hart and CLINT only for `mpfs*` and `beaglev-fire` chip names. The new
`polarfire-soc` name therefore missed the same SoC-specific CPU configuration.

This is retained as failed-attempt evidence. The follow-up adds the imported
PolarFire SoC name to the existing MPFS U54 parameter mapping before rerunning
only this target. It does not establish that the first attempt passed.
