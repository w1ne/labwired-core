# Validation Report: miv-board-additional-uarts

**Architecture:** RISC-V.

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
| `uart_net_1` | `0x70008000` | `miv_coreuart` |
| `uart_net_2` | `0x70009000` | `miv_coreuart` |
| `led0` | `0x00000000` | `led` |
| `led1` | `0x00000001` | `led` |
| `user_switch_0` | `0x00000000` | `button` |
| `user_switch_1` | `0x00000001` | `button` |
| `user_switch_2` | `0x00000002` | `button` |
| `uart` | `0x70001000` | `miv_coreuart` |
| `gpioInputs` | `0x70002000` | `miv_coregpio` |
| `gpioOutputs` | `0x70005000` | `miv_coregpio` |
| `timer0` | `0x70003000` | `miv_coretimer` |
| `timer1` | `0x70004000` | `miv_coretimer` |

**Total Peripherals:** 12