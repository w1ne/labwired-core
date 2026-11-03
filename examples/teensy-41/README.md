# Teensy 4.1 D13 LED smoke

This focused profile boots a Cortex-M7 image on the modeled MIMXRT1062 path,
ungates CCM's GPIO2 clock, and toggles the onboard D13 LED through GPIO2_IO03.
The system exposes only CCM and GPIO2. The image runs from a small DTCM slice;
the board's external flash/XIP path is not modeled.

PJRC maps D13 to GPIO_B0_03, and NXP's SVD states that the pad's reset mux
selects ALT5/GPIO2_IO03. The simulator profile maps D13 directly to GPIO2_IO03
and omits IOMUXC, so the smoke does not validate that silicon pad route. It
does not claim UART: the older LPUART6 regression fixture captures the
peripheral stream directly and does not model IOMUXC routing.

Run the committed smoke from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id teensy-41 \
  --crate firmware-teensy41-smoke \
  --target thumbv7em-none-eabi \
  --script examples/teensy-41/onboarding-smoke.yaml \
  --system configs/systems/teensy-41.yaml \
  --out-dir examples/teensy-41/evidence
```

The smoke must observe at least eight D13 GPIO transitions before the step
limit. Build and simulator artifacts are committed under `evidence/`.
