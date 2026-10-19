# Validation Report: arty_litex_vexriscv

**Architecture:** RISC-V RV32IMAC.

## 1. Dynamic Simulation Validation
**Status:** ✅ Structural Check Passed (structural-ok)

```json
{
  "system_manifest": true,
  "chip_descriptor": true,
  "flash_base_present": true,
  "ram_base_present": true
}
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `led0` | `0x00000000` | `led` |
| `led1` | `0x00000001` | `led` |
| `led2` | `0x00000002` | `led` |
| `led3` | `0x00000003` | `led` |
| `switch0` | `0x00000020` | `button` |
| `switch1` | `0x00000021` | `button` |
| `switch2` | `0x00000022` | `button` |
| `switch3` | `0x00000023` | `button` |
| `button0` | `0x00000040` | `button` |
| `button1` | `0x00000041` | `button` |
| `button2` | `0x00000042` | `button` |
| `button3` | `0x00000043` | `button` |
| `clock0` | `0xe0004800` | `litex_mmcm_csr32` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |

**Total Peripherals:** 14