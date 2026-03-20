# Validation Report: litex_tock

**Architecture:** Unknown

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
| `uart` | `0x82002000` | `litex_uart` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `timer0` | `0x82002800` | `litex_timer_csr32` |
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

**Total Peripherals:** 15