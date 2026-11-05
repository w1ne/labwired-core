# Onboarding Smoke Scoreboard

- descriptors_present: `245`
- targets_total: `245`
- run_attempted: `136`
- booted: `136/245`
- boot_and_smoke_passed: `136/245`
- not_run: `109`
- pass: `136`
- fail: `0`
- median_elapsed_ms: `906`
- median_elapsed_seconds: `0.906`
- threshold_seconds: `3600`
- threshold_met: `136/136`

> `booted` means a real run produced a result with at least one executed firmware instruction. A smoke pass proves only the assertions in that target's script, not full peripheral coverage. `not_run` means no firmware run was attempted for that descriptor.

| Target | Status | Run Attempted | Booted | Smoke Passed | Elapsed (ms) | Elapsed (s) | Threshold Met | Failure Stage | Hint | Signature |
|---|---|---|---|---|---:|---:|---|---|---|---|
| `A2_CV32E40P` | `pass` | `True` | `yes` | `yes` | `1042` | `1.042` | `True` | `n/a` | n/a | n/a |
| `a20` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `acrn_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `alif_ae822fa0e5597bs0` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ambiq-apollo4` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `andes_ae350_n25` | `pass` | `True` | `yes` | `yes` | `984` | `0.984` | `True` | `n/a` | n/a | n/a |
| `arduino_101-shield` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `arduino_nano_33_ble` | `pass` | `True` | `yes` | `yes` | `3307` | `3.307` | `True` | `n/a` | n/a | n/a |
| `arduino_uno_r4_minima` | `pass` | `True` | `yes` | `yes` | `1234` | `1.234` | `True` | `n/a` | n/a | n/a |
| `arty_litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `7445` | `7.445` | `True` | `n/a` | n/a | n/a |
| `arvsom` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `at91rm9200` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `atsamd21j17d-aft` | `pass` | `True` | `yes` | `yes` | `2777` | `2.777` | `True` | `n/a` | n/a | n/a |
| `atsamd51g19a` | `pass` | `True` | `yes` | `yes` | `2141` | `2.141` | `True` | `n/a` | n/a | n/a |
| `beaglev-fire` | `pass` | `True` | `yes` | `yes` | `697` | `0.697` | `True` | `n/a` | n/a | n/a |
| `beaglev_starlight` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `brd4116a` | `pass` | `True` | `yes` | `yes` | `790` | `0.79` | `True` | `n/a` | n/a | n/a |
| `brd4117a` | `pass` | `True` | `yes` | `yes` | `764` | `0.764` | `True` | `n/a` | n/a | n/a |
| `brd4118a` | `pass` | `True` | `yes` | `yes` | `773` | `0.773` | `True` | `n/a` | n/a | n/a |
| `brd4120a` | `pass` | `True` | `yes` | `yes` | `776` | `0.776` | `True` | `n/a` | n/a | n/a |
| `brd4121a` | `pass` | `True` | `yes` | `yes` | `771` | `0.771` | `True` | `n/a` | n/a | n/a |
| `brd4162a` | `pass` | `True` | `yes` | `yes` | `753` | `0.753` | `True` | `n/a` | n/a | n/a |
| `brd4186c` | `pass` | `True` | `yes` | `yes` | `766` | `0.766` | `True` | `n/a` | n/a | n/a |
| `brd4402a` | `pass` | `True` | `yes` | `yes` | `765` | `0.765` | `True` | `n/a` | n/a | n/a |
| `cc2538` | `pass` | `True` | `yes` | `yes` | `939` | `0.939` | `True` | `n/a` | n/a | n/a |
| `colibri-vf61` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `core-v-mcu` | `pass` | `True` | `yes` | `yes` | `768` | `0.768` | `True` | `n/a` | n/a | n/a |
| `cortex-a53-gicv2` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex-a53-gicv3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex-a53-gicv3_smp` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex-a78` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex-r52` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex-r52_smp` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex-r52_smp_4` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex_a53_console` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `cortex_a53_virtio` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `crosslink-nx-evn` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `efm32g210` | `pass` | `True` | `yes` | `yes` | `781` | `0.781` | `True` | `n/a` | n/a | n/a |
| `efm32g222` | `pass` | `True` | `yes` | `yes` | `753` | `0.753` | `True` | `n/a` | n/a | n/a |
| `efm32g232` | `pass` | `True` | `yes` | `yes` | `760` | `0.76` | `True` | `n/a` | n/a | n/a |
| `efm32g842` | `pass` | `True` | `yes` | `yes` | `732` | `0.732` | `True` | `n/a` | n/a | n/a |
| `efm32g890` | `pass` | `True` | `yes` | `yes` | `727` | `0.727` | `True` | `n/a` | n/a | n/a |
| `efm32gg942` | `pass` | `True` | `yes` | `yes` | `726` | `0.726` | `True` | `n/a` | n/a | n/a |
| `efm32gg995` | `pass` | `True` | `yes` | `yes` | `758` | `0.758` | `True` | `n/a` | n/a | n/a |
| `efm32hg350` | `pass` | `True` | `yes` | `yes` | `740` | `0.74` | `True` | `n/a` | n/a | n/a |
| `efm32jg1` | `pass` | `True` | `yes` | `yes` | `754` | `0.754` | `True` | `n/a` | n/a | n/a |
| `efm32jg12` | `pass` | `True` | `yes` | `yes` | `760` | `0.76` | `True` | `n/a` | n/a | n/a |
| `efm32lg942` | `pass` | `True` | `yes` | `yes` | `742` | `0.742` | `True` | `n/a` | n/a | n/a |
| `efm32lg995` | `pass` | `True` | `yes` | `yes` | `749` | `0.749` | `True` | `n/a` | n/a | n/a |
| `efm32pg1` | `pass` | `True` | `yes` | `yes` | `734` | `0.734` | `True` | `n/a` | n/a | n/a |
| `efm32pg12` | `pass` | `True` | `yes` | `yes` | `750` | `0.75` | `True` | `n/a` | n/a | n/a |
| `efm32tg840` | `pass` | `True` | `yes` | `yes` | `778` | `0.778` | `True` | `n/a` | n/a | n/a |
| `efm32wg995` | `pass` | `True` | `yes` | `yes` | `750` | `0.75` | `True` | `n/a` | n/a | n/a |
| `efm32zg222` | `pass` | `True` | `yes` | `yes` | `769` | `0.769` | `True` | `n/a` | n/a | n/a |
| `efr32mg1` | `pass` | `True` | `yes` | `yes` | `825` | `0.825` | `True` | `n/a` | n/a | n/a |
| `efr32mg12` | `pass` | `True` | `yes` | `yes` | `790` | `0.79` | `True` | `n/a` | n/a | n/a |
| `efr32mg13` | `pass` | `True` | `yes` | `yes` | `754` | `0.754` | `True` | `n/a` | n/a | n/a |
| `efr32mg24` | `pass` | `True` | `yes` | `yes` | `805` | `0.805` | `True` | `n/a` | n/a | n/a |
| `efr32mg26` | `pass` | `True` | `yes` | `yes` | `765` | `0.765` | `True` | `n/a` | n/a | n/a |
| `efr32xg22` | `pass` | `True` | `yes` | `yes` | `755` | `0.755` | `True` | `n/a` | n/a | n/a |
| `egis_et171` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `eos-s3` | `pass` | `True` | `yes` | `yes` | `730` | `0.73` | `True` | `n/a` | n/a | n/a |
| `eos-s3-qomu` | `pass` | `True` | `yes` | `yes` | `787` | `0.787` | `True` | `n/a` | n/a | n/a |
| `eos-s3-quickfeather` | `pass` | `True` | `yes` | `yes` | `771` | `0.771` | `True` | `n/a` | n/a | n/a |
| `ezr32hg320` | `pass` | `True` | `yes` | `yes` | `759` | `0.759` | `True` | `n/a` | n/a | n/a |
| `ezr32lg330` | `pass` | `True` | `yes` | `yes` | `729` | `0.729` | `True` | `n/a` | n/a | n/a |
| `ezr32wg330` | `pass` | `True` | `yes` | `yes` | `727` | `0.727` | `True` | `n/a` | n/a | n/a |
| `focaltech_ft9001` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `fomu` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `fsl_lx2160ardb` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `gr712rc` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `gr716` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `gr716-devboard` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ice40up5k-mdp-evn` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `imxrt1064` | `pass` | `True` | `yes` | `yes` | `806` | `0.806` | `True` | `n/a` | n/a | n/a |
| `imxrt500` | `pass` | `True` | `yes` | `yes` | `825` | `0.825` | `True` | `n/a` | n/a | n/a |
| `kendryte_k210` | `pass` | `True` | `yes` | `yes` | `789` | `0.789` | `True` | `n/a` | n/a | n/a |
| `leon3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_common` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_ibex` | `pass` | `True` | `yes` | `yes` | `747` | `0.747` | `True` | `n/a` | n/a | n/a |
| `litex_linux_vexriscv_sdcard` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_minerva` | `pass` | `True` | `yes` | `yes` | `774` | `0.774` | `True` | `n/a` | n/a | n/a |
| `litex_nexys_video_vexriscv_linux` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_picorv32` | `pass` | `True` | `yes` | `yes` | `1905` | `1.905` | `True` | `n/a` | n/a | n/a |
| `litex_tock` | `pass` | `True` | `yes` | `yes` | `767` | `0.767` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `2062` | `2.062` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `1994` | `1.994` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_micropython` | `pass` | `True` | `yes` | `yes` | `2059` | `2.059` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_smp` | `pass` | `True` | `yes` | `yes` | `908` | `0.908` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_tftp` | `pass` | `True` | `yes` | `yes` | `2203` | `2.203` | `True` | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_cfu` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_vexriscv_verilated_liteuart` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `litex_vexriscv_zephyr` | `pass` | `True` | `yes` | `yes` | `2214` | `2.214` | `True` | `n/a` | n/a | n/a |
| `litex_zephyr_vexriscv_i2s` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `lpc2294` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mars_zx3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `max32652` | `pass` | `True` | `yes` | `yes` | `959` | `0.959` | `True` | `n/a` | n/a | n/a |
| `max32652-evkit` | `pass` | `True` | `yes` | `yes` | `1172` | `1.172` | `True` | `n/a` | n/a | n/a |
| `microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mimxrt1064_evk` | `pass` | `True` | `yes` | `yes` | `929` | `0.929` | `True` | `n/a` | n/a | n/a |
| `mimxrt700_evk` | `pass` | `True` | `yes` | `yes` | `950` | `0.95` | `True` | `n/a` | n/a | n/a |
| `mimxrt798s` | `pass` | `True` | `yes` | `yes` | `996` | `0.996` | `True` | `n/a` | n/a | n/a |
| `miv` | `pass` | `True` | `yes` | `yes` | `1184` | `1.184` | `True` | `n/a` | n/a | n/a |
| `miv-board` | `pass` | `True` | `yes` | `yes` | `825` | `0.825` | `True` | `n/a` | n/a | n/a |
| `miv-board-additional-uarts` | `pass` | `True` | `yes` | `yes` | `803` | `0.803` | `True` | `n/a` | n/a | n/a |
| `miv_rv32` | `pass` | `True` | `yes` | `yes` | `1044` | `1.044` | `True` | `n/a` | n/a | n/a |
| `mpc5567` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mpfs-icicle-kit` | `pass` | `True` | `yes` | `yes` | `890` | `0.89` | `True` | `n/a` | n/a | n/a |
| `msp430f2619` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mspsim-msp430f149` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mspsim-msp430f1611` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mspsim-msp430f2617` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `mspsim-msp430f5437` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `murax_vexriscv` | `pass` | `True` | `yes` | `yes` | `960` | `0.96` | `True` | `n/a` | n/a | n/a |
| `murax_vexriscv_verilated_uart` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `nrf52840` | `pass` | `True` | `yes` | `yes` | `5021` | `5.021` | `True` | `n/a` | n/a | n/a |
| `nrf52840dk_nrf52840` | `pass` | `True` | `yes` | `yes` | `8464` | `8.464` | `True` | `n/a` | n/a | n/a |
| `nucleo_f767zi` | `pass` | `True` | `yes` | `yes` | `2452` | `2.452` | `True` | `n/a` | n/a | n/a |
| `nucleo_h753zi` | `pass` | `True` | `yes` | `yes` | `7162` | `7.162` | `True` | `n/a` | n/a | n/a |
| `nucleo_wba52cg` | `pass` | `True` | `yes` | `yes` | `4327` | `4.327` | `True` | `n/a` | n/a | n/a |
| `nuvoton_npcx9` | `pass` | `True` | `yes` | `yes` | `1221` | `1.221` | `True` | `n/a` | n/a | n/a |
| `nuvoton_npcx9m6fb_evb` | `pass` | `True` | `yes` | `yes` | `1262` | `1.262` | `True` | `n/a` | n/a | n/a |
| `nxp-frdm-imx8mplus` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `nxp-k6xf` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `nxp-mimx8ml` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `nxp-s32k388` | `pass` | `True` | `yes` | `yes` | `1277` | `1.277` | `True` | `n/a` | n/a | n/a |
| `nxp-s32k388evb` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `opentitan-earlgrey` | `pass` | `True` | `yes` | `yes` | `1044` | `1.044` | `True` | `n/a` | n/a | n/a |
| `opentitan-earlgrey-cw310` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `picosoc` | `pass` | `True` | `yes` | `yes` | `1155` | `1.155` | `True` | `n/a` | n/a | n/a |
| `polarfire-soc` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `quark-c1000` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `quark_c1000-cc2520` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ramn` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `renesas-ck_ra6m5` | `pass` | `True` | `yes` | `yes` | `1162` | `1.162` | `True` | `n/a` | n/a | n/a |
| `renesas-da14592` | `pass` | `True` | `yes` | `yes` | `950` | `0.95` | `True` | `n/a` | n/a | n/a |
| `renesas-ek_ra2e1` | `pass` | `True` | `yes` | `yes` | `935` | `0.935` | `True` | `n/a` | n/a | n/a |
| `renesas-ek_ra8m1` | `pass` | `True` | `yes` | `yes` | `979` | `0.979` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa2e1a9` | `pass` | `True` | `yes` | `yes` | `1243` | `1.243` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa2l1a` | `pass` | `True` | `yes` | `yes` | `883` | `0.883` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa4m1a` | `pass` | `True` | `yes` | `yes` | `846` | `0.846` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa6m5b` | `pass` | `True` | `yes` | `yes` | `1001` | `1.001` | `True` | `n/a` | n/a | n/a |
| `renesas-r7fa8m1a` | `pass` | `True` | `yes` | `yes` | `1034` | `1.034` | `True` | `n/a` | n/a | n/a |
| `renesas-rz_t2m_rsk` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `renesas_rz_g2l` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `renesas_rz_t2m` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ri5cy` | `pass` | `True` | `yes` | `yes` | `1711` | `1.711` | `True` | `n/a` | n/a | n/a |
| `riscv_verilated_uartlite` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `riscv_virt` | `pass` | `True` | `yes` | `yes` | `1317` | `1.317` | `True` | `n/a` | n/a | n/a |
| `s32k118` | `pass` | `True` | `yes` | `yes` | `861` | `0.861` | `True` | `n/a` | n/a | n/a |
| `sam4s` | `pass` | `True` | `yes` | `yes` | `925` | `0.925` | `True` | `n/a` | n/a | n/a |
| `sam4s16c` | `pass` | `True` | `yes` | `yes` | `907` | `0.907` | `True` | `n/a` | n/a | n/a |
| `sam4s8b` | `pass` | `True` | `yes` | `yes` | `1110` | `1.11` | `True` | `n/a` | n/a | n/a |
| `sam4s_xplained` | `pass` | `True` | `yes` | `yes` | `1028` | `1.028` | `True` | `n/a` | n/a | n/a |
| `sam_e70` | `pass` | `True` | `yes` | `yes` | `1133` | `1.133` | `True` | `n/a` | n/a | n/a |
| `sifive-fe310` | `pass` | `True` | `yes` | `yes` | `1245` | `1.245` | `True` | `n/a` | n/a | n/a |
| `sifive-fu540` | `pass` | `True` | `yes` | `yes` | `1045` | `1.045` | `True` | `n/a` | n/a | n/a |
| `sifive-fu740` | `pass` | `True` | `yes` | `yes` | `1093` | `1.093` | `True` | `n/a` | n/a | n/a |
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
| `simavr-atmega328` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
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
| `sltb001a` | `pass` | `True` | `yes` | `yes` | `1207` | `1.207` | `True` | `n/a` | n/a | n/a |
| `sltb004a` | `pass` | `True` | `yes` | `yes` | `895` | `0.895` | `True` | `n/a` | n/a | n/a |
| `slwstk6220a` | `pass` | `True` | `yes` | `yes` | `999` | `0.999` | `True` | `n/a` | n/a | n/a |
| `starfive-jh7100` | `pass` | `True` | `yes` | `yes` | `848` | `0.848` | `True` | `n/a` | n/a | n/a |
| `stk3200` | `pass` | `True` | `yes` | `yes` | `800` | `0.8` | `True` | `n/a` | n/a | n/a |
| `stk3600` | `pass` | `True` | `yes` | `yes` | `863` | `0.863` | `True` | `n/a` | n/a | n/a |
| `stk3700` | `pass` | `True` | `yes` | `yes` | `815` | `0.815` | `True` | `n/a` | n/a | n/a |
| `stk3800` | `pass` | `True` | `yes` | `yes` | `797` | `0.797` | `True` | `n/a` | n/a | n/a |
| `stm32f042` | `pass` | `True` | `yes` | `yes` | `805` | `0.805` | `True` | `n/a` | n/a | n/a |
| `stm32f072` | `pass` | `True` | `yes` | `yes` | `831` | `0.831` | `True` | `n/a` | n/a | n/a |
| `stm32f072b_discovery` | `pass` | `True` | `yes` | `yes` | `906` | `0.906` | `True` | `n/a` | n/a | n/a |
| `stm32f103` | `pass` | `True` | `yes` | `yes` | `1203` | `1.203` | `True` | `n/a` | n/a | n/a |
| `stm32f4` | `pass` | `True` | `yes` | `yes` | `1999` | `1.999` | `True` | `n/a` | n/a | n/a |
| `stm32f401cdu6-blackpill` | `pass` | `True` | `yes` | `yes` | `816` | `0.816` | `True` | `n/a` | n/a | n/a |
| `stm32f412` | `pass` | `True` | `yes` | `yes` | `971` | `0.971` | `True` | `n/a` | n/a | n/a |
| `stm32f429` | `pass` | `True` | `yes` | `yes` | `947` | `0.947` | `True` | `n/a` | n/a | n/a |
| `stm32f4_discovery` | `pass` | `True` | `yes` | `yes` | `2030` | `2.03` | `True` | `n/a` | n/a | n/a |
| `stm32f4_discovery-kit` | `pass` | `True` | `yes` | `yes` | `2074` | `2.074` | `True` | `n/a` | n/a | n/a |
| `stm32f746` | `pass` | `True` | `yes` | `yes` | `884` | `0.884` | `True` | `n/a` | n/a | n/a |
| `stm32f777` | `pass` | `True` | `yes` | `yes` | `849` | `0.849` | `True` | `n/a` | n/a | n/a |
| `stm32f7_discovery-bb` | `pass` | `True` | `yes` | `yes` | `1050` | `1.05` | `True` | `n/a` | n/a | n/a |
| `stm32h7` | `pass` | `True` | `yes` | `yes` | `2103` | `2.103` | `True` | `n/a` | n/a | n/a |
| `stm32h743` | `pass` | `True` | `yes` | `yes` | `2146` | `2.146` | `True` | `n/a` | n/a | n/a |
| `stm32h747` | `pass` | `True` | `yes` | `yes` | `2150` | `2.15` | `True` | `n/a` | n/a | n/a |
| `stm32h747-disco` | `pass` | `True` | `yes` | `yes` | `2241` | `2.241` | `True` | `n/a` | n/a | n/a |
| `stm32h753` | `pass` | `True` | `yes` | `yes` | `2301` | `2.301` | `True` | `n/a` | n/a | n/a |
| `stm32l071` | `pass` | `True` | `yes` | `yes` | `898` | `0.898` | `True` | `n/a` | n/a | n/a |
| `stm32l072` | `pass` | `True` | `yes` | `yes` | `852` | `0.852` | `True` | `n/a` | n/a | n/a |
| `stm32l151` | `pass` | `True` | `yes` | `yes` | `890` | `0.89` | `True` | `n/a` | n/a | n/a |
| `stm32l552` | `pass` | `True` | `yes` | `yes` | `1726` | `1.726` | `True` | `n/a` | n/a | n/a |
| `stm32w108` | `pass` | `True` | `yes` | `yes` | `851` | `0.851` | `True` | `n/a` | n/a | n/a |
| `stm32wba52` | `pass` | `True` | `yes` | `yes` | `2097` | `2.097` | `True` | `n/a` | n/a | n/a |
| `tegra2` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `tegra3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `tock_veer_el2_sim` | `pass` | `True` | `yes` | `yes` | `866` | `0.866` | `True` | `n/a` | n/a | n/a |
| `up_squared_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `ut32m0r500` | `pass` | `True` | `yes` | `yes` | `1017` | `1.017` | `True` | `n/a` | n/a | n/a |
| `vegaboard_ri5cy` | `pass` | `True` | `yes` | `yes` | `1401` | `1.401` | `True` | `n/a` | n/a | n/a |
| `verilated_ibex` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `versatile` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `vexpress` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `vybrid` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `x86` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `xtensa-sample-controller` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zedboard` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zolertia-firefly` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynq-7000` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp-zcu102-revA` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp-zcu102-revB` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp-zcu104` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
| `zynqmp-zcu106-vcu` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `not_configured` | Add a compatible firmware build and smoke script before claiming a boot result. | No platform-specific firmware crate and smoke script are mapped for this imported descriptor in the existing onboarding workflow; no firmware run was attempted. |
