# Validation Report: renesas-da14592

**Architecture:** ARM Cortex-M33.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `timer1` | `0x50010300` | `renesasda14_gpt` |
| `timer2` | `0x50010400` | `renesasda14_gpt` |
| `timer3` | `0x50010500` | `renesasda14_gpt` |
| `timer4` | `0x50020a00` | `renesasda14_gpt` |
| `uart0` | `0x50020000` | `renesasda14_uart` |
| `uart1` | `0x50020100` | `renesasda14_uart` |
| `gpadc` | `0x50040900` | `renesasda14_gpadc` |
| `gpio` | `0x50020600` | `renesasda14_gpio` |
| `dma` | `0x50060200` | `renesasda14_dma` |
| `i2c` | `0x50020300` | `renesasda_i2c` |
| `clock_gen` | `0x50000000` | `renesasda14_clockgenerationcontroller` |
| `xtal32m_regs` | `0x50010000` | `renesasda14_xtal32mregisters` |
| `spi` | `0x50020200` | `renesasda_spi` |
| `gp_regs` | `0x50050300` | `renesasda14_generalpurposeregisters` |

**Total Peripherals:** 14