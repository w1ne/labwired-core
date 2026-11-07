# Cortex-R52 validation status

## Current result

The imported Cortex-R52 descriptor is present, but its previous generated
`verified: true` report was not R52 evidence. It loaded a Thumb Cortex-M
fixture and ran it through LabWired's Cortex-M fallback.

The descriptor now identifies `core: cortex-r52` and keeps only its generic
flash and RAM windows. The imported PL011 and generic-timer windows were
removed from this bounded profile. The whole-platform descriptor remains
unverified.

The one-target onboarding smoke attempted a direct ARM-state integer loop at
`0x20000000`. The runner rejected the chip before executing any instructions:

```text
chip 'cortex-r52': unsupported ARM real-time core 'cortex-r52'; only cortex-r5f has a native adapter
```

The current scoreboard records this as `fail`, `run_attempted: true`,
`booted: false`, and `smoke_passed: false`. The result reports
`stop_reason: config_error` and zero executed instructions. Evidence, including
the exact result JSON, JUnit error, snapshot, and stage logs, is under
[`docs/coverage/renode-onboarding/metrics/cortex-r52/`](../coverage/renode-onboarding/metrics/cortex-r52/).

## Scope

This is local LabWired CLI evidence of an unsupported CPU core. It does not
show a Cortex-R52 boot, instruction support, peripheral behavior, Renode
execution, hosted `labwired_verify` model verification, or hardware behavior.
No CPU or peripheral stub was added. A real Cortex-R52 adapter is required
before the imported platform can boot.

The two earlier descriptor-schema attempts are retained under
[`attempt-history/cortex-r52/`](../coverage/renode-onboarding/attempt-history/cortex-r52/);
the current scoreboard row comes from the final, schema-valid run.
