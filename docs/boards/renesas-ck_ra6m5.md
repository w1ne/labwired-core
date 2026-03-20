# Validation Report: renesas-ck_ra6m5

**Architecture:** ARM Cortex-M33.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `led_red` | `0x0000000a` | `led` |
| `led_green` | `0x00000009` | `led` |
| `led_blue` | `0x00000001` | `led` |
| `button` | `0x00000004` | `button` |
| `hs3001` | `0x00000044` | `hs3001` |
| `iaq` | `0x00000032` | `zmod4xxx` |
| `oaq` | `0x00000033` | `zmod4xxx` |
| `barometer` | `0x00000063` | `icp_101xx` |
| `icm` | `0x00000068` | `icm20948` |
| `magnetometer` | `0x0000000c` | `ak09916` |
| `sysc_oscsf` | `0x4001e03c` | `pythonperipheral` |
| `gpt` | `0x40169000` | `renesasra_gpt` |
| `agt0` | `0x400e8000` | `renesasra_agt` |
| `agt1` | `0x400e8100` | `renesasra_agt` |
| `agt2` | `0x400e8200` | `renesasra_agt` |
| `agt3` | `0x400e8300` | `renesasra_agt` |
| `agt4` | `0x400e8400` | `renesasra_agt` |
| `agt5` | `0x400e8500` | `renesasra_agt` |
| `sci0` | `0x40118000` | `renesasra6m5_sci` |
| `sci1` | `0x40118100` | `renesasra6m5_sci` |
| `sci2` | `0x40118200` | `renesasra6m5_sci` |
| `sci3` | `0x40118300` | `renesasra6m5_sci` |
| `sci4` | `0x40118400` | `renesasra6m5_sci` |
| `sci5` | `0x40118500` | `renesasra6m5_sci` |
| `sci6` | `0x40118600` | `renesasra6m5_sci` |
| `sci7` | `0x40118700` | `renesasra6m5_sci` |
| `sci8` | `0x40118800` | `renesasra6m5_sci` |
| `sci9` | `0x40118900` | `renesasra6m5_sci` |
| `portMisc` | `0x40080d00` | `renesasra_gpiomisc` |
| `iic0` | `0x4009f000` | `renesasra_iic` |
| `iic1` | `0x4009f100` | `renesasra_iic` |
| `iic2` | `0x4009f200` | `renesasra_iic` |

**Total Peripherals:** 32