# LabWired GitHub Action (`w1ne/labwired-core@v1`)

Zero-cost, drop-in hardware & firmware verification for public GitHub repositories.

- **Drop-in Wokwi `diagram.json`** — run an existing Wokwi project unmodified (automatic board & peripheral translation, ERC, digital-twin lowering, and firmware simulation).
- **Drop-in KiCad `.kicad_sch` / `.kicad_pcb`** — import without hand-conversion, run ERC + real copper PCB DRC, surface actionable questions for unmapped footprints, lower to a runnable twin, and execute firmware assertions.
- **Deterministic PR Summary & `Verified by LabWired` Badge** — posts `ERC 0/0 · DRC 0/0 · X/X firmware assertions passed`, a shareable replay link, and a live SVG status badge.

## Quick Start (Drop-In)

Add `.github/workflows/labwired.yml` to any repository containing a Wokwi `diagram.json`, a KiCad `.kicad_pcb`/`.kicad_sch`, or a LabWired test script:

```yaml
name: LabWired Verify
on: [pull_request, push]

permissions:
  contents: read
  pull-requests: write
  id-token: write

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: w1ne/labwired-core@v1
```

No API keys or secrets are required for public repositories.

### Explicit Inputs Example (Wokwi or KiCad + Firmware)

```yaml
- uses: w1ne/labwired-core@v1
  with:
    diagram: examples/ci-wokwi-dropin/diagram.json
    script: examples/ci-wokwi-dropin/tests/labwired.yml
```

```yaml
- uses: w1ne/labwired-core@v1
  with:
    kicad_pcb: examples/ci-kicad-dropin/handheld-carrier.kicad_pcb
    script: examples/ci-kicad-dropin/tests/labwired.yml
```

## README Badge

```markdown
[![Verified by LabWired](https://api.labwired.com/v1/ci/badge/OWNER/REPO.svg)](https://app.labwired.com/ci/repo/OWNER/REPO)
```
