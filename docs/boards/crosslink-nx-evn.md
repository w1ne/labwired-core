# Validation Report: crosslink-nx-evn

**Architecture:** Soft RISC-V / FPGA.

## 1. Dynamic Simulation Validation
**Status:** ❌ Failed (missing-memory-map-or-manifest)

```json
{
  "system_manifest": true,
  "chip_descriptor": true,
  "flash_base_present": false,
  "ram_base_present": true
}
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `ctrl` | `0xf0000000` | `litex_soc_controller` |
| `uart` | `0xf0002000` | `litex_uart` |
| `timer0` | `0xf0002800` | `litex_timer` |
| `leds` | `0xf0003000` | `litex_controlandstatus` |
| `led0` | `0x00000000` | `led` |
| `led1` | `0x00000001` | `led` |
| `led2` | `0x00000002` | `led` |
| `led3` | `0x00000003` | `led` |
| `led4` | `0x00000004` | `led` |
| `led5` | `0x00000005` | `led` |
| `led6` | `0x00000006` | `led` |
| `led7` | `0x00000007` | `led` |
| `led8` | `0x00000008` | `led` |
| `led9` | `0x00000009` | `led` |
| `led10` | `0x0000000a` | `led` |
| `led11` | `0x0000000b` | `led` |
| `led12` | `0x0000000c` | `led` |
| `led13` | `0x0000000d` | `led` |

**Total Peripherals:** 18