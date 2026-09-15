# External Components (Teensy 4.1)

No required external simulated components for minimal deterministic smoke.

The onboarding path uses on-chip peripherals only:
1. CCM (`imx_ccm`)
2. IOMUXC (`imx_iomuxc` sticky stub)
3. GPIO2 (`imxrt`)
4. LPUART6 (`nxp_lpuart` — Serial1)
5. SysTick
6. FlexSPI stub window (no XIP)

## On-board devices intentionally omitted

Teensy 4.1 carriers include external QSPI flash (FlexSPI XIP @ `0x60000000`), USB, Ethernet PHY, SD, and optional PSRAM. Those are **out of scope** here — smoke links into DTCM only and treats FlexSPI as a stub MMIO window.

## Adding an external device (I²C / SPI sensor, EEPROM, etc.)

See [`examples/demo-blinky/`](../demo-blinky/README.md) — that example is
the canonical reference for the `external_devices` attach pattern. Before
copying, check that the bus you need is actually modeled for this chip.
At time of writing the Teensy 4.1 model covers CCM / IOMUXC / GPIO2 /
LPUART6 / SysTick / FlexSPI stub only.
