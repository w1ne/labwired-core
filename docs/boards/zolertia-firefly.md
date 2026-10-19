# Validation Report: zolertia-firefly

**Architecture:** ARM Cortex-M3.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:39.866581Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:39.867784Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/zolertia-firefly.yaml
[2m2026-10-18T14:40:39.868771Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'cc2538_gpio' (peripheral 'gpioA') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:39.868803Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'cc2538_gpio' (peripheral 'gpioB') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:39.868814Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'cc2538_gpio' (peripheral 'gpioC') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:39.868824Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'cc2538_gpio' (peripheral 'gpioD') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:39.868949Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:39.869013Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:39.869020Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:39.869024Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:39.869942Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:39.870000Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:39.870037Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:39.874349Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 1880338.22
[2m2026-10-18T14:40:39.878953Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:39.879028Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x454
[2m2026-10-18T14:40:39.879058Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 20000
[2m2026-10-18T14:40:39.879090Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20004
[2m2026-10-18T14:40:39.879117Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 1982466.47
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `cc1200` | `0x00000000` | `cc1200` |
| `uDma` | `0x400ff000` | `udma` |
| `uart0` | `0x4000c000` | `pl011` |
| `watchdog` | `0x400d5000` | `cc2538watchdog` |
| `sleepTimer` | `0x400d5040` | `cc2538sleeptimer` |
| `uart1` | `0x4000d000` | `pl011` |
| `gpioA` | `0x400d9000` | `cc2538_gpio` |
| `gpioB` | `0x400da000` | `cc2538_gpio` |
| `gpioC` | `0x400db000` | `cc2538_gpio` |
| `gpioD` | `0x400dc000` | `cc2538_gpio` |
| `radio` | `0x40088000` | `cc2538rf` |
| `bitband` | `0x22000000` | `bitbanding` |
| `crypto` | `0x4008b000` | `cc2538_cryptoprocessor` |
| `flashCtrl` | `0x400d3000` | `cc2538flashcontroller` |
| `ssi0` | `0x40008000` | `cc2538_ssi` |
| `ssi1` | `0x40009000` | `cc2538_ssi` |

**Total Peripherals:** 16