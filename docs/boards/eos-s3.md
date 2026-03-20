# Validation Report: eos-s3

**Architecture:** ARM Cortex-M4F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `uart` | `0x40010000` | `pl011` |
| `dwt` | `0xe0001000` | `dwt` |
| `spt` | `0x40005c00` | `eoss3_simpleperiodictimer` |
| `adc` | `0x40005a00` | `eoss3_adc` |
| `spi` | `0x40007000` | `designware_spi` |
| `dmaSpi` | `0x40007400` | `eoss3_spi_dma` |
| `packetFifo` | `0x40002000` | `eoss3_packetfifo` |
| `systemDma` | `0x4000c000` | `udma` |
| `systemDmaBridge` | `0x4000d000` | `eoss3_systemdmabridge` |
| `ffe` | `0x4004a000` | `eoss3_flexiblefusionengine` |
| `i2cMaster0` | `0x00000000` | `opencoresi2c` |
| `i2cMaster1` | `0x00000001` | `opencoresi2c` |
| `voice` | `0x40015000` | `eoss3_voice` |
| `powerMgmt` | `0x40004400` | `pythonperipheral` |

**Total Peripherals:** 14