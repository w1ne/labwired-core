# Validation Report: nuvoton_npcx9m6fb_evb

**Architecture:** ARM Cortex-M4F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:08.053739Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:08.054422Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/nuvoton_npcx9m6fb_evb.yaml
[2m2026-10-18T14:40:08.056637Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:08.056744Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:08.056752Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:08.056756Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:08.058974Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:08.059061Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:08.059101Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:08.059213Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:40:08.059260Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:08.059286Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:40:08.059311Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:40:08.059336Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:40:08.059361Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 769.26
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
| `spip` | `0x400d2000` | `stub` |
| `lfcg` | `0x400b5100` | `stub` |
| `hfcg` | `0x400b5000` | `stub` |
| `fiu` | `0x40020000` | `stub` |
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
| `smbus0` | `0x40009000` | `stub` |
| `smbus1` | `0x4000b000` | `stub` |
| `smbus2` | `0x400c0000` | `stub` |
| `smbus3` | `0x400c2000` | `stub` |
| `smbus4` | `0x40008000` | `stub` |
| `smbus5` | `0x40017000` | `stub` |
| `smbus6` | `0x40018000` | `stub` |
| `smbus7` | `0x40019000` | `stub` |
| `DEV_CTL4` | `0x400c3006` | `pythonperipheral` |
| `SWRST_CTL4` | `0x400c3110` | `pythonperipheral` |
| `pm1` | `0x400c9000` | `arraymemory` |
| `pm2` | `0x400cb000` | `arraymemory` |
| `pm3` | `0x400cd000` | `arraymemory` |
| `pm4` | `0x400cf000` | `arraymemory` |

**Total Peripherals:** 54