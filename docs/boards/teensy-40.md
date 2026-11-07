# Teensy 4.0 (NXP MIMXRT1062) — D13 GPIO smoke

The focused board profile runs a MIMXRT1062 Cortex-M7 smoke image, ungates
GPIO2 through CCM, and toggles the onboard D13 red LED via GPIO2_IO03. The
image runs from a small DTCM slice; the board's external flash/XIP path is not
modeled.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/mimxrt1062-teensy4-gpio-smoke.yaml`](../../configs/chips/mimxrt1062-teensy4-gpio-smoke.yaml) |
| System yaml        | [`configs/systems/teensy-40.yaml`](../../configs/systems/teensy-40.yaml) |
| Smoke crate        | `firmware-teensy4-smoke`                                            |
| Evidence           | [`examples/teensy-40/evidence/`](../../examples/teensy-40/evidence/) |
| Tier               | **smoke-manual** — GPIO behavior in the local model; no silicon diff |

## What the smoke exercises

The firmware enables the GPIO2 clock in CCM, configures GPIO2_IO03 as an
output, and repeatedly toggles D13. PJRC's pinout card identifies the onboard
red LED, and the board mapping assigns pin 13 to GPIO_B0_03 / GPIO2_IO03.
IOMUXC is absent, so this simulator profile maps D13 directly to GPIO2_IO03;
the smoke does not validate the silicon pad mux or its reset route.

## Scope limits

The system declares only the tested D13 LED path. It does not claim IOMUXC
alternate-function routing, FlexSPI/XIP, USB, serial, audio, SDIO, CAN,
Ethernet, or other board features. No zero-answering peripheral stubs are
loaded by this focused profile. No physical-board capture is claimed.

## Vendor status

SparkFun listed the Teensy 4.0 in stock at $23.80 during the 2026-11-04 live
check. NXP lists the matching MIMXRT1062DVL6B as Active. The exact URLs and
wording are recorded in
[`examples/teensy-40/evidence/vendor-status.md`](../../examples/teensy-40/evidence/vendor-status.md).

## Reproduce

See [`examples/teensy-40/VALIDATION.md`](../../examples/teensy-40/VALIDATION.md)
for the smoke command and acceptance criteria.
