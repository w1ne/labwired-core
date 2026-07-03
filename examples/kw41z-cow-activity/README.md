# KW41Z CowManager — scriptable input stimulus

The FRDM-KW41Z + FXOS8700 accelerometer + Nokia-5110 LCD "cow" demo, driven by
a **declarative input stimulus** instead of a one-shot seeded pose.

`labwired test` schema **1.2** adds a `stimuli:` track: drive a component's
input channel to a value when a trigger fires, then keep running. It's the
declarative, CI-reproducible sibling of the browser's live input sliders and
of the generic `Machine::set_input` an agent calls over MCP — all three reach
the sensor through the one `SimInput` path in `crates/core/src/sim_input.rs`,
so there is no cow- or FXOS-specific code in the runner.

## Run

```sh
# Grazing baseline — no input driven, stays CALM.
labwired test --script examples/kw41z-cow-activity/calm.yaml

# Shake it mid-run — CALM until 3 M cycles, then X=+2 g flips it to ACTIVE.
labwired test --script examples/kw41z-cow-activity/stimulus-shake.yaml
```

## The `stimuli` block

```yaml
schema_version: "1.2"
stimuli:
  - target: { component: fxos8700, channel: x }   # component is advisory;
    trigger: !after_cycles { cycles: 3000000 }     # resolution is by channel key
    value: 2.0                                      # engineering unit (g here)
```

- `target.channel` — the `SimInput` channel key (`x`/`y`/`z` on an accel,
  `distance` on an HC-SR04, `temperature` on an NTC …). `target.component` is
  an advisory hint; a stimulus resolves to the **unique** device exposing the
  channel, and an ambiguous channel is a run error rather than a silent guess.
- `trigger` — reuses the fault-trigger vocabulary. Wired today: `at_start`
  (default) and `!after_cycles { cycles: N }`. The register-access triggers
  (`on_write` / `on_read`) are rejected at validation for stimuli until the
  input path grows a write hook.
- `value` — the value in the channel's engineering unit; each device owns its
  unit→raw conversion, so the YAML is silicon-agnostic.

## Why it matters

The same script is three things at once: a demo an author can read, a
regression the merge gate runs, and the payload an agent (e.g. ChatGPT via the
LabWired MCP server) hands over to steer a run — "boot, run to steady state,
shake the sensor, confirm the cow went ACTIVE."
