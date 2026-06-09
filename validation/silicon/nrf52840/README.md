# nRF52840 silicon-validation corpus

Reset-state register captures from **real nRF52840 silicon** (Seeed XIAO nRF52840
Sense, SWD via ST-LINK V2 + OpenOCD 0.12.0). This is the ground-truth evidence that
backs the silicon-verified reset values in `core/configs/chips/nrf52840.yaml` and the
`nrf52_conformance` gate.

**Why this lives in the private repo:** per the moat boundary
(`docs/strategy/2026-06-06-moat-refinement-simulation-incumbents.md`), model code →
public `labwired-core`; the silicon-validation *evidence pipeline* → private. A code
fork gets the models but not the provenance. The capture **script** is public
(`core/scripts/hw-capture-nrf52840.sh`); its **output** (these traces) is the private
corpus.

## Contents

- `hw-capture-<timestamp>/registers.txt` — labelled `mdw` dump of FICR identity,
  Cortex-M4 system registers, and the notable reset registers of every promoted
  peripheral, captured after `reset halt`.
- `hw-capture-<timestamp>/st-info.txt` — probe info.

## Reproduce

```bash
core/scripts/hw-capture-nrf52840.sh          # writes to core/fixtures/ (gitignored scratch)
# then curate the complete run into validation/silicon/nrf52840/ and commit here
```

## Provenance — capture 20260609-131244

- Board: Seeed XIAO nRF52840 Sense
- Probe: ST-LINK V2 (STLINK V2J37S7), target voltage 3.302 V
- FICR `INFO.PART` = `0x00052840`, `INFO.VARIANT` = `0x41414430` ("AAD0")
- `DEVICEID` = `707dc298 940d8a73`
- Date: 2026-06-09
