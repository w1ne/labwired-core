# Teensy 4.0 onboarding record

## Board and live status

The Teensy 4.0 and the current MIMXRT1062DVL6B orderable were checked live on
2026-11-04. SparkFun listed the board in stock and NXP listed the exact MCU as
Active. Source URLs and the vendor wording are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Modeled board scope

[`configs/systems/teensy-40.yaml`](../../configs/systems/teensy-40.yaml) binds
the board to the focused MIMXRT1062 model at
[`mimxrt1062-teensy4-gpio-smoke.yaml`](../../configs/chips/mimxrt1062-teensy4-gpio-smoke.yaml).
The model exposes CCM and GPIO2. The smoke enables GPIO2 and toggles D13 /
GPIO2_IO03, which the board pin mapping identifies as the onboard red LED.

The image is linked into the DTCM slice used by this simulator profile; this
does not test the board's external QSPI/FlexSPI boot path. IOMUXC is also
omitted, so D13 is mapped directly to GPIO2_IO03 and the smoke does not prove
the silicon pad mux or reset routing. USB, serial, audio, SDIO, CAN, Ethernet,
and other board features remain outside this profile.

## Smoke evidence

The local simulator smoke requires at least eight GPIO2 pin 3 transitions and
stops at the configured step limit. Build and simulator output are committed
under [`evidence/`](evidence/). This is model evidence, not a physical-board
test.
