# Host sccache configuration failure

- Attempt: 2026-11-04 00:51 UTC full imported inventory refresh.
- Result: all 46 configured smokes stopped at `build_cli`; no firmware build or simulator run was attempted.
- Cause: `/home/andrii/.cargo/config.toml` configured `build.rustc-wrapper` as the missing `/home/andrii/.cargo/bin/sccache`.
- Per-target metrics, summaries, and CLI build logs are archived here. This is a runner setup failure, not a target boot result.
