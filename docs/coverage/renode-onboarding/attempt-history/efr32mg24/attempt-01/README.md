# EFR32MG24 attempt 01

- Outcome: failed before firmware compilation; no simulator run was attempted.
- Stage: `build_firmware`.
- Cause: `firmware-efr32mg24-smoke` had not yet been added to the explicit Cargo workspace member list.
- This is setup evidence only. It is not a platform boot failure and is not counted as one in the scoreboard.
- The workspace member was added before the retry; the later result is recorded under `metrics/efr32mg24/`.
