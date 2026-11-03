# Teensy 4.1 onboarding record

## Board and live status

The Teensy 4.1 and its exact MCU, MIMXRT1062DVJ6B, were listed for sale and
Active during the 2026-11-03 live check. The vendor URLs and observed wording
are recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Modeled board scope

The focused system in [`configs/systems/teensy-41.yaml`](../../configs/systems/teensy-41.yaml)
uses [`mimxrt1062-teensy41-gpio-smoke.yaml`](../../configs/chips/mimxrt1062-teensy41-gpio-smoke.yaml)
and exposes the CCM clock controller plus GPIO2. The smoke firmware enables
GPIO2 and toggles D13 / GPIO2_IO03. PJRC maps D13 to GPIO_B0_03, and the held
NXP vendor SVD says the mux register resets to `0x5`, selecting ALT5/GPIO2_IO03.
In this focused simulator profile, IOMUXC is absent and D13 is mapped directly
to GPIO2_IO03. The smoke validates the GPIO model output; it does not validate
the silicon's pad mux or reset routing.

No UART or IOMUXC alternate-function behavior is claimed. The older `imxrt1064`
LPUART6 run remains an internal model regression fixture; its UART output is
captured directly and does not prove a physical Teensy Serial1 route. USB,
Ethernet, SD, FlexSPI/XIP, and optional PSRAM are also outside this profile.

## Smoke evidence

The local simulator smoke requires at least eight GPIO transitions on pin
`gpio2:3` and stops at the configured step limit. The committed build and
simulation output are under [`evidence/`](evidence/). This is model evidence,
not a physical-board test.

## Sources

- PJRC [Teensy 4.1 product page](https://www.pjrc.com/store/teensy41.html)
  identifies the board and states that SparkFun manufactures it and sells it
  directly.
- NXP [MIMXRT1062DVJ6B product page](https://www.nxp.com/part/MIMXRT1062DVJ6B)
  is the exact MCU part page. The package-quality table lists this orderable
  Active; live board and chip availability is captured in
  [`evidence/vendor-status.md`](evidence/vendor-status.md).
- NXP CMSIS-SVD `MIMXRT1062.svd`, rev. 1.0, vendor claim, SHA-256
  `4b017b09946d58262acb5757735b350aa9272dd08cf727af652ac9ea43e02d5a`,
  `$.peripherals.IOMUXC.registers.SW_MUX_CTL_PAD_GPIO_B0_03`: stated reset
  value `0x5`; ALT5 selects GPIO2_IO03.
- PJRC [Teensy 4.1 pinout reference](https://www.pjrc.com/teensy/card11a_rev4_web.pdf)
  labels the onboard LED on pin 13. PJRC's
  [product page](https://www.pjrc.com/store/teensy41.html) says the pin 13 LED
  is driven when the pin is logic HIGH.
