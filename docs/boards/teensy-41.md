# Teensy 4.1 (NXP MIMXRT1062) — D13 GPIO smoke

The focused board profile uses the MIMXRT1062 identity shared with Teensy 4.0
and exposes only the CCM clock controller and GPIO2 path used by the onboard
D13 LED. The smoke image runs from a small DTCM slice; it does not simulate
external flash or XIP boot.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/mimxrt1062-teensy4-gpio-smoke.yaml`](../../configs/chips/mimxrt1062-teensy4-gpio-smoke.yaml) |
| System yaml        | [`configs/systems/teensy-41.yaml`](../../configs/systems/teensy-41.yaml) |
| Smoke crate        | `firmware-teensy4-smoke`                                             |
| Evidence           | [`examples/teensy-41/evidence/`](../../examples/teensy-41/evidence/) |
| Tier               | **smoke-manual** — GPIO behavior in the local model; no silicon diff |

## What the smoke exercises

Bare-metal firmware enables the GPIO2 clock in CCM, configures GPIO2_IO03 as an
output, and repeatedly toggles the D13 LED through `DR_TOGGLE`. PJRC maps D13
to GPIO_B0_03, and the MIMXRT1062 SVD states that its mux reset value is `0x5`
(ALT5/GPIO2_IO03). IOMUXC is absent from this simulator profile, which maps
D13 directly to GPIO2_IO03. The smoke validates the modeled GPIO output; it
does not validate the silicon pad mux or its reset route.

## Scope limits

The profile does not claim Serial1/LPUART6 pin routing, IOMUXC alternate
functions, FlexSPI/XIP, USB, Ethernet, SD, optional PSRAM, GPT, PIT, or DMA.
Those blocks are absent from this focused board descriptor. The older
`imxrt1064` LPUART smoke remains only as an internal peripheral regression
fixture; it is not evidence for the physical board's serial path. No physical
board capture is claimed.

## Vendor status

PJRC links Teensy 4.1 purchases through SparkFun, which listed the board in
stock at $31.50 during the 2026-11-03 check. NXP lists the exact
MIMXRT1062DVJ6B as Active. See the recorded live URLs and statuses in
[`examples/teensy-41/evidence/vendor-status.md`](../../examples/teensy-41/evidence/vendor-status.md).

## Reproduce

See [`examples/teensy-41/VALIDATION.md`](../../examples/teensy-41/VALIDATION.md)
for the smoke command and acceptance criteria.
