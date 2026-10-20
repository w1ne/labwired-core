# Validation Report: alif_ae822fa0e5597bs0

**Architecture:** ARM Cortex-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:39:49.928895Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:39:49.929210Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/alif_ae822fa0e5597bs0.yaml
[2m2026-10-18T14:39:49.933916Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio0') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.933959Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio1') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.933971Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio2') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.933981Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio3') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.933992Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio4') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934002Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio5') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934013Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio6') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934022Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio7') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934032Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio8') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934043Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio9') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934053Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio10') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934063Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio11') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934074Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio12') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934084Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio13') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934094Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'gpio14') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934104Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'designware_apb_gpio' (peripheral 'lpgpio') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:49.934298Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:39:49.934366Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:39:49.934373Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:39:49.934377Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:39:49.935672Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:39:49.939452Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:39:49.939475Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:39:49.939565Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:39:49.939583Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:39:49.939588Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:39:49.939592Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:39:49.939596Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:39:49.939600Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 383.22
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `gpio0` | `0x49000000` | `designware_apb_gpio` |
| `gpio1` | `0x49001000` | `designware_apb_gpio` |
| `gpio2` | `0x49002000` | `designware_apb_gpio` |
| `gpio3` | `0x49003000` | `designware_apb_gpio` |
| `gpio4` | `0x49004000` | `designware_apb_gpio` |
| `gpio5` | `0x49005000` | `designware_apb_gpio` |
| `gpio6` | `0x49006000` | `designware_apb_gpio` |
| `gpio7` | `0x49007000` | `designware_apb_gpio` |
| `gpio8` | `0x49008000` | `designware_apb_gpio` |
| `gpio9` | `0x49009000` | `designware_apb_gpio` |
| `gpio10` | `0x4900a000` | `designware_apb_gpio` |
| `gpio11` | `0x4900b000` | `designware_apb_gpio` |
| `gpio12` | `0x4900c000` | `designware_apb_gpio` |
| `gpio13` | `0x4900d000` | `designware_apb_gpio` |
| `gpio14` | `0x4900e000` | `designware_apb_gpio` |
| `lpgpio` | `0x42002000` | `designware_apb_gpio` |
| `i2c0` | `0x49010000` | `stub` |
| `i2c1` | `0x49011000` | `stub` |
| `i2c2` | `0x49012000` | `stub` |
| `i2c3` | `0x49013000` | `stub` |
| `uart0` | `0x49018000` | `ns16550` |
| `uart1` | `0x49019000` | `ns16550` |
| `uart2` | `0x4901a000` | `ns16550` |
| `uart3` | `0x4901b000` | `ns16550` |
| `uart4` | `0x4901c000` | `ns16550` |
| `uart5` | `0x4901d000` | `ns16550` |
| `uart6` | `0x4901e000` | `ns16550` |
| `uart7` | `0x4901f000` | `ns16550` |
| `fdcan` | `0x49036000` | `stub` |
| `dwt` | `0xe0001000` | `dwt` |

**Total Peripherals:** 30