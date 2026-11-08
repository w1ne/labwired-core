# Seeed Studio XIAO nRF54L15 Sense

This profile covers the Sense board, Seeed SKU 101991422. It boots a
Cortex-M33 image and exercises the active-low user LED on P2.00 through the
nRF54L GPIO2 model. The committed smoke asserts at least 16 GPIO output edges.

The system profile only claims the user LED GPIO path. The board also has an
LSM6DS3TR-C 6-axis IMU and an MSM261DGT006 digital microphone; neither device
is modeled here. In particular, the IMU shares GPIO pads with the header and
needs its switched power rail controlled through P0.01. USB/debug, UARTE,
wireless, ADC, battery measurement, charging, and microphone PDM are also
outside this profile.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id xiao-nrf54l15-sense \
  --crate firmware-xiao-nrf54l15-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/xiao-nrf54l15-sense/onboarding-smoke.yaml \
  --system configs/systems/xiao-nrf54l15-sense.yaml \
  --out-dir examples/xiao-nrf54l15-sense/evidence
```

The committed build and simulator output is under `evidence/`. Seeed's live
stock check and the component lifecycle evidence are in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- Seeed's [XIAO nRF54L15 Sense listing](https://jp.seeedstudio.com/XIAO-nRF54L15-Sense-p-6494.html)
  identifies SKU 101991422 and its stock status.
- The [Seeed board guide](https://wiki.seeedstudio.com/xiao_nrf54l15_sense_getting_started/)
  maps the user LED to P2.00 and names the onboard IMU and microphone. The
  [Sense schematic](https://files.seeedstudio.com/wiki/XIAO_nRF54L15/res/202004618_XIAO_nRF54L15_Sense_V1.2_SCH_260605.pdf.pdf)
  shows the IMU bus and switched sensor rail.
- Nordic's [nRF54L15 product catalog](https://www.nordicsemi.com/Products)
  currently lists the nRF54L15. The local chip descriptor reuses the nRF54L
  GPIO model and declares no sensor or communications peripherals.

The local simulator assumes an ideal VDD rail, so it does not model power-on
or brown-out reset. This is digital-twin evidence, not a physical board capture.
The hosted verification attempts and their compile-time limitations are
recorded in [`evidence/labwired-verification.md`](evidence/labwired-verification.md).
