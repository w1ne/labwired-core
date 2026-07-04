# Co-Simulation Plant Demo

A minimal, domain-neutral example of LabWired's co-simulation surface. It
declares one external-process model in the manifest and routes generic signal
paths into and out of it — nothing chip- or vendor-specific.

The model (`models/discrete-plant.py`) is a reduced-order discrete plant: it
counts how many of twelve channels are enabled (respecting a `disabled_channels`
scenario list) and reports that as voltage, current, and an active-channel
count. It exists to exercise the contract — manifest parsing, adapter
construction, manifest-relative model loading, and signal routing — not to model
any real device.

## Run It Through LabWired

```bash
labwired cosim-step examples/cosim-plant-demo/system.yaml \
    --set plant.channels.0.enabled=true \
    --set plant.channels.1.enabled=true \
    --set 'scenario.disabled_channels=["channel11"]'
```

`cosim-step` loads the manifest, validates the `cosim_models`, builds the
runner (resolving `./models/...` relative to the manifest), seeds the signal
store from `--set`, steps each model to its `step_ns` boundary, and prints the
routed outputs. Pass `--json` for machine-readable output.

## Adapt It

Swap `models/discrete-plant.py` for any process that reads a JSONL
`{"inputs": {...}}` line on stdin and writes `{"outputs": {...}}`. The model can
represent a motor, thermal plant, battery pack, power stage, or any external
simulator; only the manifest `inputs`/`outputs` maps change.
