# Onboarding Smoke Scoreboard

- descriptors_present: `245`
- targets_total: `245`
- run_attempted: `166`
- booted: `165/245`
- boot_and_smoke_passed: `165/245`
- not_run: `79`
- pass: `165`
- fail: `1`
- median_elapsed_ms: `1264`
- median_elapsed_seconds: `1.264`
- threshold_seconds: `3600`
- threshold_met: `166/166`

> `booted` means a real run produced a result with at least one executed firmware instruction. A smoke pass proves only the assertions in that target's script, not full peripheral coverage. `not_run` means no firmware run was attempted for that descriptor.

| Target | Status | Run Attempted | Booted | Smoke Passed | Elapsed (ms) | Elapsed (s) | Threshold Met | Failure Stage | Hint | Signature |
|---|---|---|---|---|---:|---:|---|---|---|---|
| `A2_CV32E40P` | `pass` | `True` | `yes` | `yes` | `91017` | `91.017` | `True` | `n/a` | n/a | n/a |
| `a20` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `acrn_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `alif_ae822fa0e5597bs0` | `pass` | `True` | `yes` | `yes` | `7479` | `7.479` | `True` | `n/a` | n/a | n/a |
| `ambiq-apollo4` | `pass` | `True` | `yes` | `yes` | `5634` | `5.634` | `True` | `n/a` | n/a | n/a |
| `andes_ae350_n25` | `pass` | `True` | `yes` | `yes` | `1074` | `1.074` | `True` | `n/a` | n/a | n/a |
| `arduino_101-shield` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `arduino_nano_33_ble` | `pass` | `True` | `yes` | `yes` | `2450` | `2.45` | `True` | `n/a` | n/a | n/a |
| `arduino_uno_r4_minima` | `pass` | `True` | `yes` | `yes` | `119052` | `119.052` | `True` | `n/a` | n/a | n/a |
| `arty_litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `3557` | `3.557` | `True` | `n/a` | n/a | n/a |
| `arvsom` | `pass` | `True` | `yes` | `yes` | `8226` | `8.226` | `True` | `n/a` | n/a | n/a |
| `at91rm9200` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `atsamd21j17d-aft` | `pass` | `True` | `yes` | `yes` | `1530` | `1.53` | `True` | `n/a` | n/a | n/a |
| `atsamd51g19a` | `pass` | `True` | `yes` | `yes` | `1028` | `1.028` | `True` | `n/a` | n/a | n/a |
| `beaglev-fire` | `pass` | `True` | `yes` | `yes` | `588` | `0.588` | `True` | `n/a` | n/a | n/a |
| `beaglev_starlight` | `pass` | `True` | `yes` | `yes` | `7993` | `7.993` | `True` | `n/a` | n/a | n/a |
| `brd4116a` | `pass` | `True` | `yes` | `yes` | `1101` | `1.101` | `True` | `n/a` | n/a | n/a |
| `brd4117a` | `pass` | `True` | `yes` | `yes` | `752` | `0.752` | `True` | `n/a` | n/a | n/a |
| `brd4118a` | `pass` | `True` | `yes` | `yes` | `741` | `0.741` | `True` | `n/a` | n/a | n/a |
| `brd4120a` | `pass` | `True` | `yes` | `yes` | `762` | `0.762` | `True` | `n/a` | n/a | n/a |
| `brd4121a` | `pass` | `True` | `yes` | `yes` | `741` | `0.741` | `True` | `n/a` | n/a | n/a |
| `brd4162a` | `pass` | `True` | `yes` | `yes` | `1028` | `1.028` | `True` | `n/a` | n/a | n/a |
| `brd4186c` | `pass` | `True` | `yes` | `yes` | `1007` | `1.007` | `True` | `n/a` | n/a | n/a |
| `brd4402a` | `pass` | `True` | `yes` | `yes` | `985` | `0.985` | `True` | `n/a` | n/a | n/a |
| `cc2538` | `pass` | `True` | `yes` | `yes` | `1421` | `1.421` | `True` | `n/a` | n/a | n/a |
| `colibri-vf61` | `pass` | `True` | `yes` | `yes` | `7802` | `7.802` | `True` | `n/a` | n/a | n/a |
| `core-v-mcu` | `pass` | `True` | `yes` | `yes` | `764` | `0.764` | `True` | `n/a` | n/a | n/a |
| `cortex-a53-gicv2` | `pass` | `True` | `yes` | `yes` | `12566` | `12.566` | `True` | `n/a` | n/a | n/a |
| `cortex-a53-gicv3` | `pass` | `True` | `yes` | `yes` | `10155` | `10.155` | `True` | `n/a` | n/a | n/a |
| `cortex-a53-gicv3_smp` | `pass` | `True` | `yes` | `yes` | `8231` | `8.231` | `True` | `n/a` | n/a | n/a |
| `cortex-a78` | `pass` | `True` | `yes` | `yes` | `7642` | `7.642` | `True` | `n/a` | n/a | n/a |
| `cortex-r52` | `fail` | `True` | `no` | `no` | `665` | `0.665` | `True` | `run_smoke` | CPU model gap. Add a real adapter for this core before claiming boot support. | chip 'cortex-r52': unsupported ARM real-time core 'cortex-r52'; only cortex-r5f has a native adapter |
| `cortex-r52_smp` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex-r52_smp_4` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex_a53_console` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex_a53_virtio` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `crosslink-nx-evn` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `efm32g210` | `pass` | `True` | `yes` | `yes` | `1035` | `1.035` | `True` | `n/a` | n/a | n/a |
| `efm32g222` | `pass` | `True` | `yes` | `yes` | `1031` | `1.031` | `True` | `n/a` | n/a | n/a |
| `efm32g232` | `pass` | `True` | `yes` | `yes` | `1019` | `1.019` | `True` | `n/a` | n/a | n/a |
| `efm32g842` | `pass` | `True` | `yes` | `yes` | `1033` | `1.033` | `True` | `n/a` | n/a | n/a |
| `efm32g890` | `pass` | `True` | `yes` | `yes` | `761` | `0.761` | `True` | `n/a` | n/a | n/a |
| `efm32gg942` | `pass` | `True` | `yes` | `yes` | `771` | `0.771` | `True` | `n/a` | n/a | n/a |
| `efm32gg995` | `pass` | `True` | `yes` | `yes` | `781` | `0.781` | `True` | `n/a` | n/a | n/a |
| `efm32hg350` | `pass` | `True` | `yes` | `yes` | `1017` | `1.017` | `True` | `n/a` | n/a | n/a |
| `efm32jg1` | `pass` | `True` | `yes` | `yes` | `1038` | `1.038` | `True` | `n/a` | n/a | n/a |
| `efm32jg12` | `pass` | `True` | `yes` | `yes` | `1031` | `1.031` | `True` | `n/a` | n/a | n/a |
| `efm32lg942` | `pass` | `True` | `yes` | `yes` | `1014` | `1.014` | `True` | `n/a` | n/a | n/a |
| `efm32lg995` | `pass` | `True` | `yes` | `yes` | `1009` | `1.009` | `True` | `n/a` | n/a | n/a |
| `efm32pg1` | `pass` | `True` | `yes` | `yes` | `993` | `0.993` | `True` | `n/a` | n/a | n/a |
| `efm32pg12` | `pass` | `True` | `yes` | `yes` | `1001` | `1.001` | `True` | `n/a` | n/a | n/a |
| `efm32tg840` | `pass` | `True` | `yes` | `yes` | `990` | `0.99` | `True` | `n/a` | n/a | n/a |
| `efm32wg995` | `pass` | `True` | `yes` | `yes` | `1031` | `1.031` | `True` | `n/a` | n/a | n/a |
| `efm32zg222` | `pass` | `True` | `yes` | `yes` | `1011` | `1.011` | `True` | `n/a` | n/a | n/a |
| `efr32mg1` | `pass` | `True` | `yes` | `yes` | `980` | `0.98` | `True` | `n/a` | n/a | n/a |
| `efr32mg12` | `pass` | `True` | `yes` | `yes` | `734` | `0.734` | `True` | `n/a` | n/a | n/a |
| `efr32mg13` | `pass` | `True` | `yes` | `yes` | `989` | `0.989` | `True` | `n/a` | n/a | n/a |
| `efr32mg24` | `pass` | `True` | `yes` | `yes` | `741` | `0.741` | `True` | `n/a` | n/a | n/a |
| `efr32mg26` | `pass` | `True` | `yes` | `yes` | `1013` | `1.013` | `True` | `n/a` | n/a | n/a |
| `efr32xg22` | `pass` | `True` | `yes` | `yes` | `727` | `0.727` | `True` | `n/a` | n/a | n/a |
| `egis_et171` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `eos-s3` | `pass` | `True` | `yes` | `yes` | `736` | `0.736` | `True` | `n/a` | n/a | n/a |
| `eos-s3-qomu` | `pass` | `True` | `yes` | `yes` | `1009` | `1.009` | `True` | `n/a` | n/a | n/a |
| `eos-s3-quickfeather` | `pass` | `True` | `yes` | `yes` | `754` | `0.754` | `True` | `n/a` | n/a | n/a |
| `ezr32hg320` | `pass` | `True` | `yes` | `yes` | `995` | `0.995` | `True` | `n/a` | n/a | n/a |
| `ezr32lg330` | `pass` | `True` | `yes` | `yes` | `1047` | `1.047` | `True` | `n/a` | n/a | n/a |
| `ezr32wg330` | `pass` | `True` | `yes` | `yes` | `738` | `0.738` | `True` | `n/a` | n/a | n/a |
| `focaltech_ft9001` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `fomu` | `pass` | `True` | `yes` | `yes` | `1011` | `1.011` | `True` | `n/a` | n/a | n/a |
| `fsl_lx2160ardb` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `gr712rc` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `gr716` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `gr716-devboard` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ice40up5k-mdp-evn` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `imxrt1064` | `pass` | `True` | `yes` | `yes` | `996` | `0.996` | `True` | `n/a` | n/a | n/a |
| `imxrt500` | `pass` | `True` | `yes` | `yes` | `1239` | `1.239` | `True` | `n/a` | n/a | n/a |
| `kendryte_k210` | `pass` | `True` | `yes` | `yes` | `1497` | `1.497` | `True` | `n/a` | n/a | n/a |
| `leon3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_common` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_ibex` | `pass` | `True` | `yes` | `yes` | `1518` | `1.518` | `True` | `n/a` | n/a | n/a |
| `litex_linux_vexriscv_sdcard` | `pass` | `True` | `yes` | `yes` | `1760` | `1.76` | `True` | `n/a` | n/a | n/a |
| `litex_microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_minerva` | `pass` | `True` | `yes` | `yes` | `1546` | `1.546` | `True` | `n/a` | n/a | n/a |
| `litex_nexys_video_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `2269` | `2.269` | `True` | `n/a` | n/a | n/a |
| `litex_picorv32` | `pass` | `True` | `yes` | `yes` | `2265` | `2.265` | `True` | `n/a` | n/a | n/a |
| `litex_tock` | `pass` | `True` | `yes` | `yes` | `1077` | `1.077` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `2357` | `2.357` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `2316` | `2.316` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_micropython` | `pass` | `True` | `yes` | `yes` | `2405` | `2.405` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_smp` | `pass` | `True` | `yes` | `yes` | `1051` | `1.051` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_tftp` | `pass` | `True` | `yes` | `yes` | `2302` | `2.302` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_cfu` | `pass` | `True` | `yes` | `yes` | `2255` | `2.255` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_liteuart` | `pass` | `True` | `yes` | `yes` | `2364` | `2.364` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_zephyr` | `pass` | `True` | `yes` | `yes` | `1993` | `1.993` | `True` | `n/a` | n/a | n/a |
| `litex_zephyr_vexriscv_i2s` | `pass` | `True` | `yes` | `yes` | `1043` | `1.043` | `True` | `n/a` | n/a | n/a |
| `lpc2294` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mars_zx3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `max32652` | `pass` | `True` | `yes` | `yes` | `762` | `0.762` | `True` | `n/a` | n/a | n/a |
| `max32652-evkit` | `pass` | `True` | `yes` | `yes` | `1040` | `1.04` | `True` | `n/a` | n/a | n/a |
| `microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mimxrt1064_evk` | `pass` | `True` | `yes` | `yes` | `788` | `0.788` | `True` | `n/a` | n/a | n/a |
| `mimxrt700_evk` | `pass` | `True` | `yes` | `yes` | `1149` | `1.149` | `True` | `n/a` | n/a | n/a |
| `mimxrt798s` | `pass` | `True` | `yes` | `yes` | `1094` | `1.094` | `True` | `n/a` | n/a | n/a |
| `miv` | `pass` | `True` | `yes` | `yes` | `804` | `0.804` | `True` | `n/a` | n/a | n/a |
| `miv-board` | `pass` | `True` | `yes` | `yes` | `1083` | `1.083` | `True` | `n/a` | n/a | n/a |
| `miv-board-additional-uarts` | `pass` | `True` | `yes` | `yes` | `1018` | `1.018` | `True` | `n/a` | n/a | n/a |
| `miv_rv32` | `pass` | `True` | `yes` | `yes` | `758` | `0.758` | `True` | `n/a` | n/a | n/a |
| `mpc5567` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mpfs-icicle-kit` | `pass` | `True` | `yes` | `yes` | `564` | `0.564` | `True` | `n/a` | n/a | n/a |
| `msp430f2619` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mspsim-msp430f149` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mspsim-msp430f1611` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mspsim-msp430f2617` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mspsim-msp430f5437` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `murax_vexriscv` | `pass` | `True` | `yes` | `yes` | `1032` | `1.032` | `True` | `n/a` | n/a | n/a |
| `murax_vexriscv_verilated_uart` | `pass` | `True` | `yes` | `yes` | `754` | `0.754` | `True` | `n/a` | n/a | n/a |
| `nrf52840` | `pass` | `True` | `yes` | `yes` | `2733` | `2.733` | `True` | `n/a` | n/a | n/a |
| `nrf52840dk_nrf52840` | `pass` | `True` | `yes` | `yes` | `2156` | `2.156` | `True` | `n/a` | n/a | n/a |
| `nucleo_f767zi` | `pass` | `True` | `yes` | `yes` | `1266` | `1.266` | `True` | `n/a` | n/a | n/a |
| `nucleo_h753zi` | `pass` | `True` | `yes` | `yes` | `2391` | `2.391` | `True` | `n/a` | n/a | n/a |
| `nucleo_wba52cg` | `pass` | `True` | `yes` | `yes` | `2299` | `2.299` | `True` | `n/a` | n/a | n/a |
| `nuvoton_npcx9` | `pass` | `True` | `yes` | `yes` | `1314` | `1.314` | `True` | `n/a` | n/a | n/a |
| `nuvoton_npcx9m6fb_evb` | `pass` | `True` | `yes` | `yes` | `813` | `0.813` | `True` | `n/a` | n/a | n/a |
| `nxp-frdm-imx8mplus` | `pass` | `True` | `yes` | `yes` | `8747` | `8.747` | `True` | `n/a` | n/a | n/a |
| `nxp-k6xf` | `pass` | `True` | `yes` | `yes` | `796` | `0.796` | `True` | `n/a` | n/a | n/a |
| `nxp-mimx8ml` | `pass` | `True` | `yes` | `yes` | `8193` | `8.193` | `True` | `n/a` | n/a | n/a |
| `nxp-s32k388` | `pass` | `True` | `yes` | `yes` | `1435` | `1.435` | `True` | `n/a` | n/a | n/a |
| `nxp-s32k388evb` | `pass` | `True` | `yes` | `yes` | `7818` | `7.818` | `True` | `n/a` | n/a | n/a |
| `opentitan-earlgrey` | `pass` | `True` | `yes` | `yes` | `841` | `0.841` | `True` | `n/a` | n/a | n/a |
| `opentitan-earlgrey-cw310` | `pass` | `True` | `yes` | `yes` | `1349` | `1.349` | `True` | `n/a` | n/a | n/a |
| `picosoc` | `pass` | `True` | `yes` | `yes` | `1288` | `1.288` | `True` | `n/a` | n/a | n/a |
| `polarfire-soc` | `pass` | `True` | `yes` | `yes` | `87319` | `87.319` | `True` | `n/a` | n/a | n/a |
| `quark-c1000` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `quark_c1000-cc2520` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ramn` | `pass` | `True` | `yes` | `yes` | `1208` | `1.208` | `True` | `n/a` | n/a | n/a |
| `renesas-ck_ra6m5` | `pass` | `True` | `yes` | `yes` | `1426` | `1.426` | `True` | `n/a` | n/a | n/a |
| `renesas-da14592` | `pass` | `True` | `yes` | `yes` | `1360` | `1.36` | `True` | `n/a` | n/a | n/a |
| `renesas-ek_ra2e1` | `pass` | `True` | `yes` | `yes` | `1325` | `1.325` | `True` | `n/a` | n/a | n/a |
| `renesas-ek_ra8m1` | `pass` | `True` | `yes` | `yes` | `1406` | `1.406` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa2e1a9` | `pass` | `True` | `yes` | `yes` | `1332` | `1.332` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa2l1a` | `pass` | `True` | `yes` | `yes` | `1366` | `1.366` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa4m1a` | `pass` | `True` | `yes` | `yes` | `942` | `0.942` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa6m5b` | `pass` | `True` | `yes` | `yes` | `1295` | `1.295` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa8m1a` | `pass` | `True` | `yes` | `yes` | `1267` | `1.267` | `True` | `n/a` | n/a | n/a |
| `renesas-rz_t2m_rsk` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `renesas_rz_g2l` | `pass` | `True` | `yes` | `yes` | `7765` | `7.765` | `True` | `n/a` | n/a | n/a |
| `renesas_rz_t2m` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ri5cy` | `pass` | `True` | `yes` | `yes` | `1953` | `1.953` | `True` | `n/a` | n/a | n/a |
| `riscv_verilated_uartlite` | `pass` | `True` | `yes` | `yes` | `1297` | `1.297` | `True` | `n/a` | n/a | n/a |
| `riscv_virt` | `pass` | `True` | `yes` | `yes` | `1408` | `1.408` | `True` | `n/a` | n/a | n/a |
| `s32k118` | `pass` | `True` | `yes` | `yes` | `1307` | `1.307` | `True` | `n/a` | n/a | n/a |
| `sam4s` | `pass` | `True` | `yes` | `yes` | `1373` | `1.373` | `True` | `n/a` | n/a | n/a |
| `sam4s16c` | `pass` | `True` | `yes` | `yes` | `1263` | `1.263` | `True` | `n/a` | n/a | n/a |
| `sam4s8b` | `pass` | `True` | `yes` | `yes` | `1183` | `1.183` | `True` | `n/a` | n/a | n/a |
| `sam4s_xplained` | `pass` | `True` | `yes` | `yes` | `1435` | `1.435` | `True` | `n/a` | n/a | n/a |
| `sam_e70` | `pass` | `True` | `yes` | `yes` | `1378` | `1.378` | `True` | `n/a` | n/a | n/a |
| `sifive-fe310` | `pass` | `True` | `yes` | `yes` | `1355` | `1.355` | `True` | `n/a` | n/a | n/a |
| `sifive-fu540` | `pass` | `True` | `yes` | `yes` | `1354` | `1.354` | `True` | `n/a` | n/a | n/a |
| `sifive-fu740` | `pass` | `True` | `yes` | `yes` | `1456` | `1.456` | `True` | `n/a` | n/a | n/a |
| `simavr-at90usb162` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega128` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega1280` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega1281` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega1284p` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega128rfa1` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega128rfr2` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega16` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega164` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega168` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega169p` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega16m1` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega2560` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega32` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega324` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega328` | `pass` | `True` | `yes` | `yes` | `14574` | `14.574` | `True` | `n/a` | n/a | n/a |
| `simavr-atmega328pb` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega32u4` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega48` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega644` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega64m1` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega8` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-atmega88` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny13` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny1634` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny2313` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny2313a` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny24` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny25` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny4313` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny44` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny45` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny84` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `simavr-attiny85` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `sltb001a` | `pass` | `True` | `yes` | `yes` | `1013` | `1.013` | `True` | `n/a` | n/a | n/a |
| `sltb004a` | `pass` | `True` | `yes` | `yes` | `809` | `0.809` | `True` | `n/a` | n/a | n/a |
| `slwstk6220a` | `pass` | `True` | `yes` | `yes` | `1383` | `1.383` | `True` | `n/a` | n/a | n/a |
| `starfive-jh7100` | `pass` | `True` | `yes` | `yes` | `1328` | `1.328` | `True` | `n/a` | n/a | n/a |
| `stk3200` | `pass` | `True` | `yes` | `yes` | `1334` | `1.334` | `True` | `n/a` | n/a | n/a |
| `stk3600` | `pass` | `True` | `yes` | `yes` | `1303` | `1.303` | `True` | `n/a` | n/a | n/a |
| `stk3700` | `pass` | `True` | `yes` | `yes` | `852` | `0.852` | `True` | `n/a` | n/a | n/a |
| `stk3800` | `pass` | `True` | `yes` | `yes` | `1328` | `1.328` | `True` | `n/a` | n/a | n/a |
| `stm32f042` | `pass` | `True` | `yes` | `yes` | `1332` | `1.332` | `True` | `n/a` | n/a | n/a |
| `stm32f072` | `pass` | `True` | `yes` | `yes` | `1410` | `1.41` | `True` | `n/a` | n/a | n/a |
| `stm32f072b_discovery` | `pass` | `True` | `yes` | `yes` | `1352` | `1.352` | `True` | `n/a` | n/a | n/a |
| `stm32f103` | `pass` | `True` | `yes` | `yes` | `1600` | `1.6` | `True` | `n/a` | n/a | n/a |
| `stm32f4` | `pass` | `True` | `yes` | `yes` | `2282` | `2.282` | `True` | `n/a` | n/a | n/a |
| `stm32f401cdu6-blackpill` | `pass` | `True` | `yes` | `yes` | `1175` | `1.175` | `True` | `n/a` | n/a | n/a |
| `stm32f412` | `pass` | `True` | `yes` | `yes` | `1343` | `1.343` | `True` | `n/a` | n/a | n/a |
| `stm32f429` | `pass` | `True` | `yes` | `yes` | `964` | `0.964` | `True` | `n/a` | n/a | n/a |
| `stm32f4_discovery` | `pass` | `True` | `yes` | `yes` | `1905` | `1.905` | `True` | `n/a` | n/a | n/a |
| `stm32f4_discovery-kit` | `pass` | `True` | `yes` | `yes` | `1961` | `1.961` | `True` | `n/a` | n/a | n/a |
| `stm32f746` | `pass` | `True` | `yes` | `yes` | `774` | `0.774` | `True` | `n/a` | n/a | n/a |
| `stm32f777` | `pass` | `True` | `yes` | `yes` | `764` | `0.764` | `True` | `n/a` | n/a | n/a |
| `stm32f7_discovery-bb` | `pass` | `True` | `yes` | `yes` | `746` | `0.746` | `True` | `n/a` | n/a | n/a |
| `stm32h7` | `pass` | `True` | `yes` | `yes` | `1943` | `1.943` | `True` | `n/a` | n/a | n/a |
| `stm32h743` | `pass` | `True` | `yes` | `yes` | `1917` | `1.917` | `True` | `n/a` | n/a | n/a |
| `stm32h747` | `pass` | `True` | `yes` | `yes` | `1906` | `1.906` | `True` | `n/a` | n/a | n/a |
| `stm32h747-disco` | `pass` | `True` | `yes` | `yes` | `1950` | `1.95` | `True` | `n/a` | n/a | n/a |
| `stm32h753` | `pass` | `True` | `yes` | `yes` | `1979` | `1.979` | `True` | `n/a` | n/a | n/a |
| `stm32l071` | `pass` | `True` | `yes` | `yes` | `1077` | `1.077` | `True` | `n/a` | n/a | n/a |
| `stm32l072` | `pass` | `True` | `yes` | `yes` | `1070` | `1.07` | `True` | `n/a` | n/a | n/a |
| `stm32l151` | `pass` | `True` | `yes` | `yes` | `1070` | `1.07` | `True` | `n/a` | n/a | n/a |
| `stm32l552` | `pass` | `True` | `yes` | `yes` | `1940` | `1.94` | `True` | `n/a` | n/a | n/a |
| `stm32w108` | `pass` | `True` | `yes` | `yes` | `990` | `0.99` | `True` | `n/a` | n/a | n/a |
| `stm32wba52` | `pass` | `True` | `yes` | `yes` | `1705` | `1.705` | `True` | `n/a` | n/a | n/a |
| `tegra2` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `tegra3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `tock_veer_el2_sim` | `pass` | `True` | `yes` | `yes` | `1001` | `1.001` | `True` | `n/a` | n/a | n/a |
| `up_squared_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ut32m0r500` | `pass` | `True` | `yes` | `yes` | `1004` | `1.004` | `True` | `n/a` | n/a | n/a |
| `vegaboard_ri5cy` | `pass` | `True` | `yes` | `yes` | `1404` | `1.404` | `True` | `n/a` | n/a | n/a |
| `verilated_ibex` | `pass` | `True` | `yes` | `yes` | `757` | `0.757` | `True` | `n/a` | n/a | n/a |
| `versatile` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `vexpress` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `vybrid` | `pass` | `True` | `yes` | `yes` | `7594` | `7.594` | `True` | `n/a` | n/a | n/a |
| `x86` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `xtensa-sample-controller` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zedboard` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zolertia-firefly` | `pass` | `True` | `yes` | `yes` | `1036` | `1.036` | `True` | `n/a` | n/a | n/a |
| `zynq-7000` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp-zcu102-revA` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp-zcu102-revB` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp-zcu104` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp-zcu106-vcu` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
