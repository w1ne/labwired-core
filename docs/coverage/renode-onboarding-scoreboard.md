# Onboarding Smoke Scoreboard

- descriptors_present: `245`
- targets_total: `245`
- run_attempted: `225`
- booted: `225/245`
- boot_and_smoke_passed: `225/245`
- not_run: `20`
- pass: `225`
- fail: `0`
- pass_loading_zero_answering_windows: `0/225`
- pass_observable_assertions: `225/225`
- pass_stop_reason_only: `0/225`
- pass_depth_unknown: `0/225`
- median_elapsed_ms: `1154`
- median_elapsed_seconds: `1.154`
- threshold_seconds: `3600`
- threshold_met: `225/225`

> `booted` means a real run produced a result with at least one executed firmware instruction. A smoke pass proves only the assertions in that target's script, not full peripheral coverage. `not_run` means no firmware run was attempted for that descriptor. `Zero Windows` lists the peripherals in the chip the target's system actually loads that are allowlisted stubs (`crates/core/src/bus/known_stubs.rs`) and answer reads with zeros; a pass with zero windows did not exercise those blocks. `Smoke Depth` is `stop_reason_only` when the script asserts nothing but `expected_stop_reason` (the CPU stepped to its budget without faulting; no UART, memory or GPIO result is checked) and `observable` when it also checks a UART, memory, GPIO or fidelity result.

| Target | Status | Run Attempted | Booted | Smoke Passed | Elapsed (ms) | Elapsed (s) | Threshold Met | Smoke Depth | Zero Windows | Failure Stage | Hint | Signature |
|---|---|---|---|---|---:|---:|---|---|---|---|---|---|
| `A2_CV32E40P` | `pass` | `True` | `yes` | `yes` | `91017` | `91.017` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `a20` | `pass` | `True` | `yes` | `yes` | `270406` | `270.406` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `acrn_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 1: `hpet:hpet` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no x86-64 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `alif_ae822fa0e5597bs0` | `pass` | `True` | `yes` | `yes` | `9943` | `9.943` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `ambiq-apollo4` | `pass` | `True` | `yes` | `yes` | `1154` | `1.154` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `andes_ae350_n25` | `pass` | `True` | `yes` | `yes` | `1074` | `1.074` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `arduino_101-shield` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 7: `led:led`, `button:button`, `lm74:ti_lm74`, `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `arduino_nano_33_ble` | `pass` | `True` | `yes` | `yes` | `2450` | `2.45` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `arduino_uno_r4_minima` | `pass` | `True` | `yes` | `yes` | `119052` | `119.052` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `arty_litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `3557` | `3.557` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `arvsom` | `pass` | `True` | `yes` | `yes` | `897` | `0.897` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `at91rm9200` | `pass` | `True` | `yes` | `yes` | `368699` | `368.699` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `atsamd21j17d-aft` | `pass` | `True` | `yes` | `yes` | `1530` | `1.53` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `atsamd51g19a` | `pass` | `True` | `yes` | `yes` | `1028` | `1.028` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `beaglev-fire` | `pass` | `True` | `yes` | `yes` | `657` | `0.657` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `beaglev_starlight` | `pass` | `True` | `yes` | `yes` | `7993` | `7.993` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `brd4116a` | `pass` | `True` | `yes` | `yes` | `1101` | `1.101` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `brd4117a` | `pass` | `True` | `yes` | `yes` | `752` | `0.752` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `brd4118a` | `pass` | `True` | `yes` | `yes` | `741` | `0.741` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `brd4120a` | `pass` | `True` | `yes` | `yes` | `762` | `0.762` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `brd4121a` | `pass` | `True` | `yes` | `yes` | `741` | `0.741` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `brd4162a` | `pass` | `True` | `yes` | `yes` | `1028` | `1.028` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `brd4186c` | `pass` | `True` | `yes` | `yes` | `1007` | `1.007` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `brd4402a` | `pass` | `True` | `yes` | `yes` | `985` | `0.985` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cc2538` | `pass` | `True` | `yes` | `yes` | `186459` | `186.459` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `colibri-vf61` | `pass` | `True` | `yes` | `yes` | `10349` | `10.349` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `core-v-mcu` | `pass` | `True` | `yes` | `yes` | `764` | `0.764` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv2` | `pass` | `True` | `yes` | `yes` | `572` | `0.572` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv3` | `pass` | `True` | `yes` | `yes` | `534` | `0.534` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv3_smp` | `pass` | `True` | `yes` | `yes` | `546` | `0.546` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex-a78` | `pass` | `True` | `yes` | `yes` | `586` | `0.586` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex-r52` | `pass` | `True` | `yes` | `yes` | `652` | `0.652` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex-r52_smp` | `pass` | `True` | `yes` | `yes` | `632` | `0.632` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex-r52_smp_4` | `pass` | `True` | `yes` | `yes` | `677` | `0.677` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex_a53_console` | `pass` | `True` | `yes` | `yes` | `108603` | `108.603` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `cortex_a53_virtio` | `pass` | `True` | `yes` | `yes` | `517` | `0.517` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `crosslink-nx-evn` | `pass` | `True` | `yes` | `yes` | `8533` | `8.533` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32g210` | `pass` | `True` | `yes` | `yes` | `1035` | `1.035` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32g222` | `pass` | `True` | `yes` | `yes` | `1031` | `1.031` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32g232` | `pass` | `True` | `yes` | `yes` | `1019` | `1.019` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32g842` | `pass` | `True` | `yes` | `yes` | `1033` | `1.033` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32g890` | `pass` | `True` | `yes` | `yes` | `761` | `0.761` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32gg942` | `pass` | `True` | `yes` | `yes` | `771` | `0.771` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32gg995` | `pass` | `True` | `yes` | `yes` | `781` | `0.781` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32hg350` | `pass` | `True` | `yes` | `yes` | `1017` | `1.017` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32jg1` | `pass` | `True` | `yes` | `yes` | `1038` | `1.038` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32jg12` | `pass` | `True` | `yes` | `yes` | `1031` | `1.031` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32lg942` | `pass` | `True` | `yes` | `yes` | `1014` | `1.014` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32lg995` | `pass` | `True` | `yes` | `yes` | `1009` | `1.009` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32pg1` | `pass` | `True` | `yes` | `yes` | `993` | `0.993` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32pg12` | `pass` | `True` | `yes` | `yes` | `1001` | `1.001` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32tg840` | `pass` | `True` | `yes` | `yes` | `990` | `0.99` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32wg995` | `pass` | `True` | `yes` | `yes` | `1031` | `1.031` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efm32zg222` | `pass` | `True` | `yes` | `yes` | `1011` | `1.011` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efr32mg1` | `pass` | `True` | `yes` | `yes` | `980` | `0.98` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efr32mg12` | `pass` | `True` | `yes` | `yes` | `734` | `0.734` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efr32mg13` | `pass` | `True` | `yes` | `yes` | `989` | `0.989` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efr32mg24` | `pass` | `True` | `yes` | `yes` | `741` | `0.741` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efr32mg26` | `pass` | `True` | `yes` | `yes` | `1013` | `1.013` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `efr32xg22` | `pass` | `True` | `yes` | `yes` | `727` | `0.727` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `egis_et171` | `pass` | `True` | `yes` | `yes` | `8152` | `8.152` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `eos-s3` | `pass` | `True` | `yes` | `yes` | `736` | `0.736` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `eos-s3-qomu` | `pass` | `True` | `yes` | `yes` | `1009` | `1.009` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `eos-s3-quickfeather` | `pass` | `True` | `yes` | `yes` | `754` | `0.754` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `ezr32hg320` | `pass` | `True` | `yes` | `yes` | `995` | `0.995` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `ezr32lg330` | `pass` | `True` | `yes` | `yes` | `1047` | `1.047` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `ezr32wg330` | `pass` | `True` | `yes` | `yes` | `738` | `0.738` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `focaltech_ft9001` | `pass` | `True` | `yes` | `yes` | `86395` | `86.395` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `fomu` | `pass` | `True` | `yes` | `yes` | `1011` | `1.011` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `fsl_lx2160ardb` | `pass` | `True` | `yes` | `yes` | `79412` | `79.412` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `gr712rc` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 6: `ftmctrl:gaisler_faulttolerantmemorycontroller`, `greth:gaislereth`, `phy:ethernetphysicallayer`, `ahbInfo:gaislerahbplugandplayinfo`, `apb1Controller:gaislerapbcontroller`, `apb2Controller:gaislerapbcontroller` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `gr716` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 0 | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `gr716-devboard` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 4: `switch0:button`, `switch1:button`, `switch2:button`, `switch3:button` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ice40up5k-mdp-evn` | `pass` | `True` | `yes` | `yes` | `8553` | `8.553` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `imxrt1064` | `pass` | `True` | `yes` | `yes` | `996` | `0.996` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `imxrt500` | `pass` | `True` | `yes` | `yes` | `1239` | `1.239` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `kendryte_k210` | `pass` | `True` | `yes` | `yes` | `1497` | `1.497` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `leon3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 6: `ahbInfo:gaislerahbplugandplayinfo`, `apbController:gaislerapbcontroller`, `esaMemController:esamemorycontroller`, `eth:gaislereth`, `phy:ethernetphysicallayer`, `led:led` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `litex_common` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 1: `phy:ethernetphysicallayer` | `not_configured` | Measure the concrete LiteX platforms that include it instead. | Shared LiteX include with no CPU of its own; it is not a runnable platform. |
| `litex_ibex` | `pass` | `True` | `yes` | `yes` | `1518` | `1.518` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_linux_vexriscv_sdcard` | `pass` | `True` | `yes` | `yes` | `1760` | `1.76` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 0 | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no OpenPOWER (Microwatt) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `litex_minerva` | `pass` | `True` | `yes` | `yes` | `1546` | `1.546` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_nexys_video_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `2269` | `2.269` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_picorv32` | `pass` | `True` | `yes` | `yes` | `2265` | `2.265` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_tock` | `pass` | `True` | `yes` | `yes` | `1077` | `1.077` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `97794` | `97.794` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `2316` | `2.316` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_micropython` | `pass` | `True` | `yes` | `yes` | `2405` | `2.405` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_smp` | `pass` | `True` | `yes` | `yes` | `1051` | `1.051` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_tftp` | `pass` | `True` | `yes` | `yes` | `2302` | `2.302` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_cfu` | `pass` | `True` | `yes` | `yes` | `2255` | `2.255` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_liteuart` | `pass` | `True` | `yes` | `yes` | `2364` | `2.364` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_zephyr` | `pass` | `True` | `yes` | `yes` | `1993` | `1.993` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `litex_zephyr_vexriscv_i2s` | `pass` | `True` | `yes` | `yes` | `1043` | `1.043` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `lpc2294` | `pass` | `True` | `yes` | `yes` | `77804` | `77.804` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `mars_zx3` | `pass` | `True` | `yes` | `yes` | `1589` | `1.589` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `max32652` | `pass` | `True` | `yes` | `yes` | `762` | `0.762` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `max32652-evkit` | `pass` | `True` | `yes` | `yes` | `1040` | `1.04` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 1: `uart:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no OpenPOWER (Microwatt) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `mimxrt1064_evk` | `pass` | `True` | `yes` | `yes` | `788` | `0.788` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `mimxrt700_evk` | `pass` | `True` | `yes` | `yes` | `1596` | `1.596` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `mimxrt798s` | `pass` | `True` | `yes` | `yes` | `1094` | `1.094` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `miv` | `pass` | `True` | `yes` | `yes` | `804` | `0.804` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `miv-board` | `pass` | `True` | `yes` | `yes` | `1083` | `1.083` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `miv-board-additional-uarts` | `pass` | `True` | `yes` | `yes` | `1018` | `1.018` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `miv_rv32` | `pass` | `True` | `yes` | `yes` | `758` | `0.758` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `mpc5567` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 3: `fec:fastethernetcontroller`, `emios:emios`, `fmpllSynsr:pythonperipheral` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no PowerPC e200z6 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `mpfs-icicle-kit` | `pass` | `True` | `yes` | `yes` | `91079` | `91.079` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `msp430f2619` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 4: `boot:arraymemory`, `ram_extended:arraymemory`, `code:arraymemory`, `info_mem:arraymemory` | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `mspsim-msp430f149` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 0 | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `mspsim-msp430f1611` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 0 | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `mspsim-msp430f2617` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 0 | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `mspsim-msp430f5437` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 0 | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `murax_vexriscv` | `pass` | `True` | `yes` | `yes` | `1032` | `1.032` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `murax_vexriscv_verilated_uart` | `pass` | `True` | `yes` | `yes` | `754` | `0.754` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nrf52840` | `pass` | `True` | `yes` | `yes` | `2733` | `2.733` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nrf52840dk_nrf52840` | `pass` | `True` | `yes` | `yes` | `113847` | `113.847` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nucleo_f767zi` | `pass` | `True` | `yes` | `yes` | `10204` | `10.204` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nucleo_h753zi` | `pass` | `True` | `yes` | `yes` | `2349` | `2.349` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nucleo_wba52cg` | `pass` | `True` | `yes` | `yes` | `2299` | `2.299` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nuvoton_npcx9` | `pass` | `True` | `yes` | `yes` | `1314` | `1.314` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nuvoton_npcx9m6fb_evb` | `pass` | `True` | `yes` | `yes` | `813` | `0.813` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nxp-frdm-imx8mplus` | `pass` | `True` | `yes` | `yes` | `533` | `0.533` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nxp-k6xf` | `pass` | `True` | `yes` | `yes` | `825` | `0.825` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nxp-mimx8ml` | `pass` | `True` | `yes` | `yes` | `7206` | `7.206` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nxp-s32k388` | `pass` | `True` | `yes` | `yes` | `74781` | `74.781` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `nxp-s32k388evb` | `pass` | `True` | `yes` | `yes` | `953` | `0.953` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `opentitan-earlgrey` | `pass` | `True` | `yes` | `yes` | `841` | `0.841` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `opentitan-earlgrey-cw310` | `pass` | `True` | `yes` | `yes` | `1349` | `1.349` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `picosoc` | `pass` | `True` | `yes` | `yes` | `1288` | `1.288` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `polarfire-soc` | `pass` | `True` | `yes` | `yes` | `666` | `0.666` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `quark-c1000` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 4: `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `quark_c1000-cc2520` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 8: `led:led`, `button:button`, `radio:cc2520`, `lm74:ti_lm74`, `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ramn` | `pass` | `True` | `yes` | `yes` | `11200` | `11.2` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-ck_ra6m5` | `pass` | `True` | `yes` | `yes` | `11818` | `11.818` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-da14592` | `pass` | `True` | `yes` | `yes` | `1360` | `1.36` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-ek_ra2e1` | `pass` | `True` | `yes` | `yes` | `7311` | `7.311` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-ek_ra8m1` | `pass` | `True` | `yes` | `yes` | `120815` | `120.815` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa2e1a9` | `pass` | `True` | `yes` | `yes` | `9510` | `9.51` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa2l1a` | `pass` | `True` | `yes` | `yes` | `7968` | `7.968` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa4m1a` | `pass` | `True` | `yes` | `yes` | `942` | `0.942` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa6m5b` | `pass` | `True` | `yes` | `yes` | `8999` | `8.999` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa8m1a` | `pass` | `True` | `yes` | `yes` | `899` | `0.899` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas-rz_t2m_rsk` | `pass` | `True` | `yes` | `yes` | `676` | `0.676` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas_rz_g2l` | `pass` | `True` | `yes` | `yes` | `95819` | `95.819` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `renesas_rz_t2m` | `pass` | `True` | `yes` | `yes` | `181038` | `181.038` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `ri5cy` | `pass` | `True` | `yes` | `yes` | `1953` | `1.953` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `riscv_verilated_uartlite` | `pass` | `True` | `yes` | `yes` | `1297` | `1.297` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `riscv_virt` | `pass` | `True` | `yes` | `yes` | `1408` | `1.408` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `s32k118` | `pass` | `True` | `yes` | `yes` | `1432` | `1.432` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sam4s` | `pass` | `True` | `yes` | `yes` | `1373` | `1.373` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sam4s16c` | `pass` | `True` | `yes` | `yes` | `1263` | `1.263` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sam4s8b` | `pass` | `True` | `yes` | `yes` | `1183` | `1.183` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sam4s_xplained` | `pass` | `True` | `yes` | `yes` | `116496` | `116.496` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sam_e70` | `pass` | `True` | `yes` | `yes` | `1378` | `1.378` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sifive-fe310` | `pass` | `True` | `yes` | `yes` | `1355` | `1.355` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sifive-fu540` | `pass` | `True` | `yes` | `yes` | `1354` | `1.354` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sifive-fu740` | `pass` | `True` | `yes` | `yes` | `1456` | `1.456` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-at90usb162` | `pass` | `True` | `yes` | `yes` | `220040` | `220.04` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128` | `pass` | `True` | `yes` | `yes` | `247064` | `247.064` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1280` | `pass` | `True` | `yes` | `yes` | `7863` | `7.863` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1281` | `pass` | `True` | `yes` | `yes` | `645` | `0.645` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1284p` | `pass` | `True` | `yes` | `yes` | `710` | `0.71` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128rfa1` | `pass` | `True` | `yes` | `yes` | `705` | `0.705` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128rfr2` | `pass` | `True` | `yes` | `yes` | `737` | `0.737` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega16` | `pass` | `True` | `yes` | `yes` | `151490` | `151.49` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega164` | `pass` | `True` | `yes` | `yes` | `147705` | `147.705` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega168` | `pass` | `True` | `yes` | `yes` | `216946` | `216.946` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega169p` | `pass` | `True` | `yes` | `yes` | `10783` | `10.783` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega16m1` | `pass` | `True` | `yes` | `yes` | `808` | `0.808` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega2560` | `pass` | `True` | `yes` | `yes` | `11541` | `11.541` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega32` | `pass` | `True` | `yes` | `yes` | `92624` | `92.624` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega324` | `pass` | `True` | `yes` | `yes` | `12120` | `12.12` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega328` | `pass` | `True` | `yes` | `yes` | `14574` | `14.574` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega328pb` | `pass` | `True` | `yes` | `yes` | `9330` | `9.33` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega32u4` | `pass` | `True` | `yes` | `yes` | `121577` | `121.577` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega48` | `pass` | `True` | `yes` | `yes` | `2172` | `2.172` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega644` | `pass` | `True` | `yes` | `yes` | `622` | `0.622` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega64m1` | `pass` | `True` | `yes` | `yes` | `811` | `0.811` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega8` | `pass` | `True` | `yes` | `yes` | `120400` | `120.4` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega88` | `pass` | `True` | `yes` | `yes` | `1058` | `1.058` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny13` | `pass` | `True` | `yes` | `yes` | `78119` | `78.119` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny1634` | `pass` | `True` | `yes` | `yes` | `127948` | `127.948` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny2313` | `pass` | `True` | `yes` | `yes` | `661` | `0.661` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny2313a` | `pass` | `True` | `yes` | `yes` | `673` | `0.673` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny24` | `pass` | `True` | `yes` | `yes` | `591` | `0.591` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny25` | `pass` | `True` | `yes` | `yes` | `554` | `0.554` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny4313` | `pass` | `True` | `yes` | `yes` | `701` | `0.701` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny44` | `pass` | `True` | `yes` | `yes` | `521` | `0.521` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny45` | `pass` | `True` | `yes` | `yes` | `530` | `0.53` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny84` | `pass` | `True` | `yes` | `yes` | `571` | `0.571` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny85` | `pass` | `True` | `yes` | `yes` | `534` | `0.534` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sltb001a` | `pass` | `True` | `yes` | `yes` | `1013` | `1.013` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `sltb004a` | `pass` | `True` | `yes` | `yes` | `809` | `0.809` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `slwstk6220a` | `pass` | `True` | `yes` | `yes` | `1383` | `1.383` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `starfive-jh7100` | `pass` | `True` | `yes` | `yes` | `1328` | `1.328` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stk3200` | `pass` | `True` | `yes` | `yes` | `1334` | `1.334` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stk3600` | `pass` | `True` | `yes` | `yes` | `1303` | `1.303` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stk3700` | `pass` | `True` | `yes` | `yes` | `852` | `0.852` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stk3800` | `pass` | `True` | `yes` | `yes` | `1328` | `1.328` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f042` | `pass` | `True` | `yes` | `yes` | `1332` | `1.332` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f072` | `pass` | `True` | `yes` | `yes` | `1410` | `1.41` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f072b_discovery` | `pass` | `True` | `yes` | `yes` | `9087` | `9.087` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f103` | `pass` | `True` | `yes` | `yes` | `116069` | `116.069` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f4` | `pass` | `True` | `yes` | `yes` | `2282` | `2.282` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f401cdu6-blackpill` | `pass` | `True` | `yes` | `yes` | `1175` | `1.175` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f412` | `pass` | `True` | `yes` | `yes` | `1343` | `1.343` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f429` | `pass` | `True` | `yes` | `yes` | `964` | `0.964` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f4_discovery` | `pass` | `True` | `yes` | `yes` | `1905` | `1.905` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f4_discovery-kit` | `pass` | `True` | `yes` | `yes` | `1961` | `1.961` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f746` | `pass` | `True` | `yes` | `yes` | `774` | `0.774` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f777` | `pass` | `True` | `yes` | `yes` | `764` | `0.764` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32f7_discovery-bb` | `pass` | `True` | `yes` | `yes` | `746` | `0.746` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32h7` | `pass` | `True` | `yes` | `yes` | `1943` | `1.943` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32h743` | `pass` | `True` | `yes` | `yes` | `1917` | `1.917` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32h747` | `pass` | `True` | `yes` | `yes` | `1906` | `1.906` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32h747-disco` | `pass` | `True` | `yes` | `yes` | `1950` | `1.95` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32h753` | `pass` | `True` | `yes` | `yes` | `1979` | `1.979` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32l071` | `pass` | `True` | `yes` | `yes` | `1077` | `1.077` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32l072` | `pass` | `True` | `yes` | `yes` | `1070` | `1.07` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32l151` | `pass` | `True` | `yes` | `yes` | `1070` | `1.07` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32l552` | `pass` | `True` | `yes` | `yes` | `1940` | `1.94` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32w108` | `pass` | `True` | `yes` | `yes` | `7494` | `7.494` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `stm32wba52` | `pass` | `True` | `yes` | `yes` | `2429` | `2.429` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `tegra2` | `pass` | `True` | `yes` | `yes` | `1154` | `1.154` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `tegra3` | `pass` | `True` | `yes` | `yes` | `1043` | `1.043` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `tock_veer_el2_sim` | `pass` | `True` | `yes` | `yes` | `1001` | `1.001` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `up_squared_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 2: `hpet:hpet`, `pci:pythonperipheral` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no x86-64 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ut32m0r500` | `pass` | `True` | `yes` | `yes` | `1004` | `1.004` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `vegaboard_ri5cy` | `pass` | `True` | `yes` | `yes` | `1404` | `1.404` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `verilated_ibex` | `pass` | `True` | `yes` | `yes` | `757` | `0.757` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `versatile` | `pass` | `True` | `yes` | `yes` | `154313` | `154.313` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `vexpress` | `pass` | `True` | `yes` | `yes` | `1062` | `1.062` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `vybrid` | `pass` | `True` | `yes` | `yes` | `1044` | `1.044` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `x86` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 1: `hpet:hpet` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (x86) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `xtensa-sample-controller` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | 0 | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no Xtensa sample controller (not ESP32) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `zedboard` | `pass` | `True` | `yes` | `yes` | `1906` | `1.906` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `zolertia-firefly` | `pass` | `True` | `yes` | `yes` | `1036` | `1.036` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `zynq-7000` | `pass` | `True` | `yes` | `yes` | `138530` | `138.53` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `zynqmp` | `pass` | `True` | `yes` | `yes` | `246519` | `246.519` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu102-revA` | `pass` | `True` | `yes` | `yes` | `607` | `0.607` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu102-revB` | `pass` | `True` | `yes` | `yes` | `727` | `0.727` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu104` | `pass` | `True` | `yes` | `yes` | `595` | `0.595` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu106-vcu` | `pass` | `True` | `yes` | `yes` | `575` | `0.575` | `True` | `observable` | 0 | `n/a` | n/a | n/a |
