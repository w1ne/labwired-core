# Seeed Studio XIAO RP2350 smoke

This board profile runs firmware on the RP2350 and exercises the yellow
onboard USER LED through GPIO25/D19.

| Board function | Model path | Status |
|---|---|---|
| Yellow USER LED, active-low | GPIO25 → SIO | Modeled and smoke-tested |

The chip profile is intentionally narrow: RP2350 memory, IO_BANK0 pad mux, and
SIO GPIO are declared. The board's RGB LED (PIO/WS2812), USB, battery-measurement
path, the remaining header pins, and the second M33/Hazard3 execution modes are
not represented as supported board behavior.

The broad `configs/chips/rp2350.yaml` descriptor still declares `POWMAN` and
`TBMAN` as zero-response stubs. This XIAO smoke profile omits both windows; the
firmware does not access either one.

Run the onboarding smoke from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id seeed-xiao-rp2350 \
  --crate firmware-xiao-rp2350-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/seeed-xiao-rp2350/io-smoke.yaml \
  --system configs/systems/seeed-xiao-rp2350.yaml \
  --out-dir examples/seeed-xiao-rp2350/evidence \
  --firmware-build-dir crates/firmware-xiao-rp2350-smoke
```

The committed run logs and simulation output are under `evidence/`. Vendor
availability and the board LED mapping source are recorded in
[`vendor-status.md`](vendor-status.md).
