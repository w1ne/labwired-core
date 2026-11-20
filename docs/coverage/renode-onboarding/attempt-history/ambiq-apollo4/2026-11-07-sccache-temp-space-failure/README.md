# CLI build attempt blocked by host temp space

The target-only Apollo4 pipeline stopped at `build_cli` before building firmware
or starting the simulator. `sccache` could not create a temporary directory
under `/home/andrii/.hermes/cache/scratch/` and returned `No space left on device`
while compiling `thiserror v1.0.69` (2026-11-07 19:06:35–19:08:56 UTC).

The runner's log and metric files were replaced by the retry's files. The
failure is recorded here from the captured runner output; it is an infrastructure
attempt, not an Apollo4 boot or smoke result, and is not counted in the generated
scoreboard. The retry disabled `RUSTC_WRAPPER`, built in tmpfs, corrected the
smoke YAML's checkout-relative `target/` path, and passed the same target-only
smoke.
