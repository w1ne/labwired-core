# Environment-world assertion early-stop implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Let an environment YAML opt into the same durable, assertion-based completion contract as the single-machine runner, so a successful multi-node CI run stops after its assertions have remained true for a settling window instead of always consuming `max_steps`.

**Architecture:** Keep the behavior opt-in and retain the existing default (`false`). Accept the three existing `TestLimits` fields for `inputs.env`, evaluate the already-supported node-qualified memory assertions after each successful world round, and emit `AssertionsPassed` only after the configured minimum and settling window. Runtime failures and safety limits remain higher-precedence stops. Release the behavior as v0.19.1 and point the public action/default/docs at that release.

**Tech Stack:** Rust, `labwired-config`, `labwired-cli` environment runner, GitHub Actions release archives, YAML contract tests.

## Tasks

### 1. Specify acceptance in tests

- [ ] Add a configuration test proving environment YAML accepts and round-trips all three assertion-completion limit fields.
- [ ] Remove only those fields from the existing unsupported-environment limit matrix; retain all genuinely unsupported controls.
- [ ] Add a CLI black-box world test with a small `min_steps` and `settle_steps`, asserting `status=pass`, `stop_reason=assertions_passed`, and a step count below `max_steps`.
- [ ] Add/retain a runtime-stop regression that proves a node fault wins over early completion.

### 2. Implement the world completion contract

- [ ] Stop rejecting `stop_when_assertions_pass`, `stop_when_assertions_pass_settle_steps`, and `stop_when_assertions_pass_min_steps` in `EnvTestScript::validate`.
- [ ] In `run_world`, evaluate assertions after a completed round and after runtime/safety checks.
- [ ] Latch the first all-pass round only once `min_steps` has been reached; reset the latch if an assertion regresses.
- [ ] Stop with `StopReason::AssertionsPassed` after the all-pass duration reaches `settle_steps`.

### 3. Publish a coherent v0.19.1 runner contract

- [ ] Document environment assertion completion alongside the existing YAML limits reference.
- [ ] Change the public action default to `v0.19.1`.
- [ ] Pin Core consumer examples and the static release-runner verifier to the immutable action commit that carries that default.
- [ ] Run focused config/CLI tests, formatting/lints, contract verification, and a real two-node smoke before merge.

### 4. Consume and prove it downstream

- [ ] Tag/release v0.19.1 only after Core checks are green.
- [ ] Update UDSLib’s YAML to opt in, repin the public action/version, and extend its static contract to protect environment wiring and the no-secret guarantee.
- [ ] Run the real H5 UDS workflow; preserve its generated report artifact as the landing-page proof and update only the article links.
