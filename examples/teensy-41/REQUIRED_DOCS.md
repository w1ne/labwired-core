# Required Source Documents (Teensy 4.1)

## MCU Datasheet (authoritative)

1. NXP i.MX RT1060 Processor Reference Manual — **IMXRT1060RM** (CCM, IOMUXC, GPIO DR_TOGGLE, LPUART):
   https://www.nxp.com/docs/en/reference-manual/IMXRT1060RM.pdf
2. CMSIS device header **MIMXRT1062.h** (peripheral bases: LPUART6 `0x40198000`, GPIO2 `0x401BC000`, CCM `0x400FC000`, IOMUXC `0x401F8000`).

## Board Pinout / BSP

1. PJRC Teensy 4.1 technical information / pinout:
   https://www.pjrc.com/store/teensy41.html
2. Zephyr `teensy4` board docs (pin 13 = GPIO2_3 / LED; Serial1 = LPUART6 on pins 0/1).

## Address Cross-Check Only (not a source of truth)

1. Renode `imxrt1064` / `mimxrt1064_evk` platform YAMLs may be used to **cross-check** base addresses. Do not treat Renode peripheral semantics as authoritative for LabWired models.
2. Teensy 4.1 silicon is MIMXRT1062; this LabWired chip id `imxrt1064` is the Renode RT1064 cousin (GPIO/LPUART/CCM class) — not a claim of 4MB SiP flash.
