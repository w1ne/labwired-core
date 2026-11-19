# Renesas DA14592 smoke attempt history

## Attempt 01

The first local runner invocation failed during `build_cli` because this
worktree's Cargo configuration named a missing `/home/andrii/.cargo/bin/sccache`
wrapper. The firmware was not built and no platform run was attempted. The
metrics and log are retained here to distinguish that host setup failure from a
DA14592 simulation failure.

Rerunning with `RUSTC_WRAPPER=` bypassed the absent wrapper. The passing result
is recorded under `../../metrics/renesas-da14592/` and counted by the current
scoreboard.
