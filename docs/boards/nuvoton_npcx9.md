# Validation Report: nuvoton_npcx9

**Architecture:** ARM Cortex-M4F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `dwt` | `0xe0001000` | `dwt` |
| `cr_uart1` | `0x400e0000` | `npcx_uart` |
| `cr_uart2` | `0x400e2000` | `npcx_uart` |
| `cr_uart3` | `0x400e4000` | `npcx_uart` |
| `cr_uart4` | `0x400e6000` | `npcx_uart` |
| `itim32_1` | `0x400b0000` | `npcx_itim` |
| `itim32_2` | `0x400b2000` | `npcx_itim` |
| `itim32_3` | `0x400b4000` | `npcx_itim` |
| `itim32_4` | `0x400b6000` | `npcx_itim` |
| `itim32_5` | `0x400b8000` | `npcx_itim` |
| `itim32_6` | `0x400ba000` | `npcx_itim` |
| `itim64` | `0x400be000` | `npcx_itim` |
| `image_type` | `0x4000c009` | `arraymemory` |
| `twd` | `0x400d8000` | `npcx_twd` |
| `mtc` | `0x400b7000` | `npcx_mtc` |
| `mdma1` | `0x40011100` | `npcx_mdma` |
| `mdma2` | `0x40011200` | `npcx_mdma` |
| `mdma3` | `0x40011300` | `npcx_mdma` |
| `mdma4` | `0x40011400` | `npcx_mdma` |
| `mdma5` | `0x40011500` | `npcx_mdma` |
| `spip` | `0x400d2000` | `npcx_spip` |
| `lfcg` | `0x400b5100` | `npcx_lfcg` |
| `hfcg` | `0x400b5000` | `npcx_hfcg` |
| `fiu` | `0x40020000` | `npcx_fiu` |
| `gpio0` | `0x40081000` | `npcx_gpio` |
| `gpio1` | `0x40083000` | `npcx_gpio` |
| `gpio2` | `0x40085000` | `npcx_gpio` |
| `gpio3` | `0x40087000` | `npcx_gpio` |
| `gpio4` | `0x40089000` | `npcx_gpio` |
| `gpio5` | `0x4008b000` | `npcx_gpio` |
| `gpio6` | `0x4008d000` | `npcx_gpio` |
| `gpio7` | `0x4008f000` | `npcx_gpio` |
| `gpio8` | `0x40091000` | `npcx_gpio` |
| `gpio9` | `0x40093000` | `npcx_gpio` |
| `gpioa` | `0x40095000` | `npcx_gpio` |
| `gpiob` | `0x40097000` | `npcx_gpio` |
| `gpioc` | `0x40099000` | `npcx_gpio` |
| `gpiod` | `0x4009b000` | `npcx_gpio` |
| `gpioe` | `0x4009d000` | `npcx_gpio` |
| `gpiof` | `0x4009f000` | `npcx_gpio` |
| `smbus0` | `0x40009000` | `npcx_smbus` |
| `smbus1` | `0x4000b000` | `npcx_smbus` |
| `smbus2` | `0x400c0000` | `npcx_smbus` |
| `smbus3` | `0x400c2000` | `npcx_smbus` |
| `smbus4` | `0x40008000` | `npcx_smbus` |
| `smbus5` | `0x40017000` | `npcx_smbus` |
| `smbus6` | `0x40018000` | `npcx_smbus` |
| `smbus7` | `0x40019000` | `npcx_smbus` |
| `DEV_CTL4` | `0x400c3006` | `pythonperipheral` |
| `SWRST_CTL4` | `0x400c3110` | `pythonperipheral` |
| `pm1` | `0x400c9000` | `arraymemory` |
| `pm2` | `0x400cb000` | `arraymemory` |
| `pm3` | `0x400cd000` | `arraymemory` |
| `pm4` | `0x400cf000` | `arraymemory` |

**Total Peripherals:** 54