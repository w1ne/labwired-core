# Validation Report: sifive-fu740

**Architecture:** RISC-V RV64GC.

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
| `uart0` | `0x10010000` | `sifive_uart` |
| `uart1` | `0x10011000` | `sifive_uart` |
| `i2c1` | `0x10030000` | `opencoresi2c` |
| `i2c2` | `0x10031000` | `opencoresi2c` |
| `qspi0` | `0x10040000` | `hifive_spi` |
| `qspi1` | `0x10041000` | `hifive_spi` |
| `qspi2` | `0x10050000` | `hifive_spi` |
| `gpio` | `0x10060000` | `sifive_gpio` |
| `ethernet` | `0x10090000` | `cadencegem` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |

**Total Peripherals:** 10