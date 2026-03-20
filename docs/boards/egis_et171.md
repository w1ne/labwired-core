# Validation Report: egis_et171

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
| `mtimer` | `0xe6000000` | `riscvmachinetimer` |
| `uart0` | `0xf0200020` | `ns16550` |
| `pit0` | `0xf0400000` | `andesatcpit100` |
| `gpio0` | `0xf0700000` | `andesatcgpio100` |
| `watchdog` | `0xf0500000` | `andesatcwdt200_watchdog` |
| `spi0` | `0xf0b00000` | `andesatcspi200` |
| `spi1` | `0xf0f00000` | `andesatcspi200` |
| `rtc0` | `0xf0600000` | `andesatcrtc100` |
| `syscon` | `0xf0100000` | `egiset171_aosmu` |
| `smu2` | `0xf0e00000` | `egiset171_smu2` |
| `crypto` | `0xe8000000` | `egiset171_crypto` |

**Total Peripherals:** 11