# Teensy 4.0 D13 GPIO smoke

This focused board profile runs the MIMXRT1062-class smoke firmware, enables
the GPIO2 clock through CCM, and toggles the onboard D13 red LED through
GPIO2_IO03. The firmware image is linked into the modeled DTCM slice; the
board's FlexSPI/XIP boot path is outside this smoke.

The board descriptor exposes only the tested D13 LED path. The simulator
profile omits IOMUXC, so it maps D13 directly to GPIO2_IO03 and does not prove
the silicon pad mux or reset route. USB, serial, audio, SDIO, CAN, Ethernet,
and other board features are not claimed.

Reproduce from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id teensy-40 \
  --crate firmware-teensy4-smoke \
  --target thumbv7em-none-eabi \
  --script examples/teensy-40/onboarding-smoke.yaml \
  --system configs/systems/teensy-40.yaml \
  --out-dir examples/teensy-40/evidence
```

The acceptance check requires at least eight GPIO2 pin 3 transitions before
the configured step limit. Build and simulation artifacts are committed under
[`evidence/`](evidence/). Vendor availability and the exact MCU status are
recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources

- PJRC [Teensy 4.0 product page](https://www.pjrc.com/store/teensy40.html)
  identifies SparkFun as the manufacturer and direct seller.
- PJRC [Teensy 4.0 pinout card](https://www.pjrc.com/teensy/card10a_rev2_web.pdf)
  labels the onboard red LED and board pinout.
- The upstream Zephyr [Teensy 4.0 board mapping](https://github.com/zephyrproject-rtos/zephyr/blob/main/boards/pjrc/teensy4/doc/index.rst)
  maps pin 13 to GPIO_B0_03 / GPIO2_IO03 and the onboard LED.
- NXP's [MIMXRT1060 package-quality table](https://www.nxp.com/products/i.MX-RT1060?tab=Package_Quality_Tab)
  lists the Teensy 4.0's MIMXRT1062DVL6B as Active.
