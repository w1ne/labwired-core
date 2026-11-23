# Onboarding Smoke Scoreboard

- descriptors_present: `245`
- targets_total: `245`
- run_attempted: `231`
- booted: `231/245`
- boot_and_smoke_passed: `231/245`
- not_run: `14`
- pass: `231`
- fail: `0`
- pass_loading_zero_answering_windows: `0/231`
- pass_observable_assertions: `231/231`
- pass_stop_reason_only: `0/231`
- pass_depth_unknown: `0/231`
- pass_claimed_peripherals_total: `1711`
- pass_claimed_peripherals_median: `4`
- median_elapsed_ms: `1897`
- median_elapsed_seconds: `1.897`
- threshold_seconds: `3600`
- threshold_met: `231/231`

> `booted` means a real run produced a result with at least one executed firmware instruction. A smoke pass proves only the assertions in that target's script, not full peripheral coverage. `not_run` means no firmware run was attempted for that descriptor. `Zero Windows` lists the peripherals in the chip the target's system actually loads that are allowlisted stubs (`crates/core/src/bus/known_stubs.rs`) and answer reads with zeros; a pass with zero windows did not exercise those blocks. `Smoke Depth` is `stop_reason_only` when the script asserts nothing but `expected_stop_reason` (the CPU stepped to its budget without faulting; no UART, memory or GPIO result is checked) and `observable` when it also checks a UART, memory, GPIO or fidelity result. `Claimed` is the number of peripheral windows the loaded chip profile declares and `Checks` the assertion kinds the smoke script uses; a smoke rarely exercises every claimed window, so a large `Claimed` next to a single `uart_contains` is a profile claim the smoke does not prove.

| Target | Status | Run Attempted | Booted | Smoke Passed | Elapsed (ms) | Elapsed (s) | Threshold Met | Smoke Depth | Claimed | Checks | Zero Windows | Failure Stage | Hint | Signature |
|---|---|---|---|---|---:|---:|---|---|---:|---|---|---|---|---|
| `A2_CV32E40P` | `pass` | `True` | `yes` | `yes` | `7053` | `7.053` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `a20` | `pass` | `True` | `yes` | `yes` | `859329` | `859.329` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `acrn_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `2` | n/a | 1: `hpet:hpet` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no x86-64 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `alif_ae822fa0e5597bs0` | `pass` | `True` | `yes` | `yes` | `1009` | `1.009` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `ambiq-apollo4` | `pass` | `True` | `yes` | `yes` | `1828` | `1.828` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `andes_ae350_n25` | `pass` | `True` | `yes` | `yes` | `1366` | `1.366` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `arduino_101-shield` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `10` | n/a | 7: `led:led`, `button:button`, `lm74:ti_lm74`, `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `arduino_nano_33_ble` | `pass` | `True` | `yes` | `yes` | `7408` | `7.408` | `True` | `observable` | `51` | `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `arduino_uno_r4_minima` | `pass` | `True` | `yes` | `yes` | `149953` | `149.953` | `True` | `observable` | `6` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `arty_litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `5026` | `5.026` | `True` | `observable` | `4` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `arvsom` | `pass` | `True` | `yes` | `yes` | `2948` | `2.948` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `at91rm9200` | `pass` | `True` | `yes` | `yes` | `2822` | `2.822` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `atsamd21j17d-aft` | `pass` | `True` | `yes` | `yes` | `2336` | `2.336` | `True` | `observable` | `14` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `atsamd51g19a` | `pass` | `True` | `yes` | `yes` | `2430` | `2.43` | `True` | `observable` | `4` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `beaglev-fire` | `pass` | `True` | `yes` | `yes` | `810` | `0.81` | `True` | `observable` | `8` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `beaglev_starlight` | `pass` | `True` | `yes` | `yes` | `4987` | `4.987` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4116a` | `pass` | `True` | `yes` | `yes` | `4920` | `4.92` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4117a` | `pass` | `True` | `yes` | `yes` | `4567` | `4.567` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4118a` | `pass` | `True` | `yes` | `yes` | `2427` | `2.427` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4120a` | `pass` | `True` | `yes` | `yes` | `3071` | `3.071` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4121a` | `pass` | `True` | `yes` | `yes` | `990` | `0.99` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4162a` | `pass` | `True` | `yes` | `yes` | `1111` | `1.111` | `True` | `observable` | `12` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `brd4186c` | `pass` | `True` | `yes` | `yes` | `925` | `0.925` | `True` | `observable` | `6` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `brd4402a` | `pass` | `True` | `yes` | `yes` | `1257` | `1.257` | `True` | `observable` | `5` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `cc2538` | `pass` | `True` | `yes` | `yes` | `3391` | `3.391` | `True` | `observable` | `24` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `colibri-vf61` | `pass` | `True` | `yes` | `yes` | `828` | `0.828` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `core-v-mcu` | `pass` | `True` | `yes` | `yes` | `1268` | `1.268` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv2` | `pass` | `True` | `yes` | `yes` | `1949` | `1.949` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv3` | `pass` | `True` | `yes` | `yes` | `1254` | `1.254` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv3_smp` | `pass` | `True` | `yes` | `yes` | `1030` | `1.03` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-a78` | `pass` | `True` | `yes` | `yes` | `930` | `0.93` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-r52` | `pass` | `True` | `yes` | `yes` | `932129` | `932.129` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-r52_smp` | `pass` | `True` | `yes` | `yes` | `781` | `0.781` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-r52_smp_4` | `pass` | `True` | `yes` | `yes` | `752` | `0.752` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex_a53_console` | `pass` | `True` | `yes` | `yes` | `3014` | `3.014` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex_a53_virtio` | `pass` | `True` | `yes` | `yes` | `895` | `0.895` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `crosslink-nx-evn` | `pass` | `True` | `yes` | `yes` | `1544` | `1.544` | `True` | `observable` | `3` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `efm32g210` | `pass` | `True` | `yes` | `yes` | `1897` | `1.897` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32g222` | `pass` | `True` | `yes` | `yes` | `1562` | `1.562` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32g232` | `pass` | `True` | `yes` | `yes` | `3242` | `3.242` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32g842` | `pass` | `True` | `yes` | `yes` | `1325` | `1.325` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32g890` | `pass` | `True` | `yes` | `yes` | `1786` | `1.786` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32gg942` | `pass` | `True` | `yes` | `yes` | `1648` | `1.648` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32gg995` | `pass` | `True` | `yes` | `yes` | `1265` | `1.265` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32hg350` | `pass` | `True` | `yes` | `yes` | `1903` | `1.903` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32jg1` | `pass` | `True` | `yes` | `yes` | `1190` | `1.19` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32jg12` | `pass` | `True` | `yes` | `yes` | `1338` | `1.338` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32lg942` | `pass` | `True` | `yes` | `yes` | `1364` | `1.364` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32lg995` | `pass` | `True` | `yes` | `yes` | `1621` | `1.621` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32pg1` | `pass` | `True` | `yes` | `yes` | `1690` | `1.69` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32pg12` | `pass` | `True` | `yes` | `yes` | `1546` | `1.546` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32tg840` | `pass` | `True` | `yes` | `yes` | `1447` | `1.447` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32wg995` | `pass` | `True` | `yes` | `yes` | `1543` | `1.543` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32zg222` | `pass` | `True` | `yes` | `yes` | `1296` | `1.296` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efr32mg1` | `pass` | `True` | `yes` | `yes` | `5402` | `5.402` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efr32mg12` | `pass` | `True` | `yes` | `yes` | `4119` | `4.119` | `True` | `observable` | `12` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efr32mg13` | `pass` | `True` | `yes` | `yes` | `3088` | `3.088` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efr32mg24` | `pass` | `True` | `yes` | `yes` | `1489` | `1.489` | `True` | `observable` | `6` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `efr32mg26` | `pass` | `True` | `yes` | `yes` | `3149` | `3.149` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `efr32xg22` | `pass` | `True` | `yes` | `yes` | `2728` | `2.728` | `True` | `observable` | `5` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `egis_et171` | `pass` | `True` | `yes` | `yes` | `1139` | `1.139` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `eos-s3` | `pass` | `True` | `yes` | `yes` | `1059` | `1.059` | `True` | `observable` | `5` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `eos-s3-qomu` | `pass` | `True` | `yes` | `yes` | `1908` | `1.908` | `True` | `observable` | `5` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `eos-s3-quickfeather` | `pass` | `True` | `yes` | `yes` | `2018` | `2.018` | `True` | `observable` | `5` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `ezr32hg320` | `pass` | `True` | `yes` | `yes` | `3234` | `3.234` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `ezr32lg330` | `pass` | `True` | `yes` | `yes` | `4470` | `4.47` | `True` | `observable` | `14` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `ezr32wg330` | `pass` | `True` | `yes` | `yes` | `1438` | `1.438` | `True` | `observable` | `14` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `focaltech_ft9001` | `pass` | `True` | `yes` | `yes` | `1678` | `1.678` | `True` | `observable` | `5` | `uart_regex`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `fomu` | `pass` | `True` | `yes` | `yes` | `1325` | `1.325` | `True` | `observable` | `1` | `memory_value` | 0 | `n/a` | n/a | n/a |
| `fsl_lx2160ardb` | `pass` | `True` | `yes` | `yes` | `1190` | `1.19` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `gr712rc` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `16` | n/a | 8: `ftmctrl:gaisler_faulttolerantmemorycontroller`, `gptimer:stub`, `grtimer:stub`, `greth:gaislereth`, `phy:ethernetphysicallayer`, `ahbInfo:gaislerahbplugandplayinfo`, `apb1Controller:gaislerapbcontroller`, `apb2Controller:gaislerapbcontroller` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `gr716` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `9` | n/a | 1: `timer0:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `gr716-devboard` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `13` | n/a | 5: `switch0:button`, `switch1:button`, `switch2:button`, `switch3:button`, `timer0:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ice40up5k-mdp-evn` | `pass` | `True` | `yes` | `yes` | `2448` | `2.448` | `True` | `observable` | `1` | `memory_value` | 0 | `n/a` | n/a | n/a |
| `imxrt1064` | `pass` | `True` | `yes` | `yes` | `313839` | `313.839` | `True` | `observable` | `5` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `imxrt500` | `pass` | `True` | `yes` | `yes` | `3827` | `3.827` | `True` | `observable` | `6` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `kendryte_k210` | `pass` | `True` | `yes` | `yes` | `1623` | `1.623` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `leon3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `9` | n/a | 7: `ahbInfo:gaislerahbplugandplayinfo`, `apbController:gaislerapbcontroller`, `esaMemController:esamemorycontroller`, `timer:stub`, `eth:gaislereth`, `phy:ethernetphysicallayer`, `led:led` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `litex_common` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `1` | n/a | 1: `phy:ethernetphysicallayer` | `not_configured` | Measure the concrete LiteX platforms that include it instead. | Shared LiteX include with no CPU of its own; it is not a runnable platform. |
| `litex_ibex` | `pass` | `True` | `yes` | `yes` | `1021` | `1.021` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_linux_vexriscv_sdcard` | `pass` | `True` | `yes` | `yes` | `1419` | `1.419` | `True` | `observable` | `1` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `2` | n/a | 1: `timer0:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no OpenPOWER (Microwatt) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `litex_minerva` | `pass` | `True` | `yes` | `yes` | `1144` | `1.144` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_nexys_video_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `2383` | `2.383` | `True` | `observable` | `2` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_picorv32` | `pass` | `True` | `yes` | `yes` | `1906` | `1.906` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_tock` | `pass` | `True` | `yes` | `yes` | `984` | `0.984` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `2134` | `2.134` | `True` | `observable` | `3` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `2059` | `2.059` | `True` | `observable` | `2` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_micropython` | `pass` | `True` | `yes` | `yes` | `2876` | `2.876` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_smp` | `pass` | `True` | `yes` | `yes` | `1395` | `1.395` | `True` | `observable` | `1` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_tftp` | `pass` | `True` | `yes` | `yes` | `2370` | `2.37` | `True` | `observable` | `2` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_cfu` | `pass` | `True` | `yes` | `yes` | `2341` | `2.341` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_liteuart` | `pass` | `True` | `yes` | `yes` | `2361` | `2.361` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_zephyr` | `pass` | `True` | `yes` | `yes` | `1949` | `1.949` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_zephyr_vexriscv_i2s` | `pass` | `True` | `yes` | `yes` | `1614` | `1.614` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `lpc2294` | `pass` | `True` | `yes` | `yes` | `992` | `0.992` | `True` | `observable` | `11` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `mars_zx3` | `pass` | `True` | `yes` | `yes` | `1595` | `1.595` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `max32652` | `pass` | `True` | `yes` | `yes` | `2605` | `2.605` | `True` | `observable` | `7` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `max32652-evkit` | `pass` | `True` | `yes` | `yes` | `4762` | `4.762` | `True` | `observable` | `7` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `1` | n/a | 1: `uart:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no OpenPOWER (Microwatt) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `mimxrt1064_evk` | `pass` | `True` | `yes` | `yes` | `10298` | `10.298` | `True` | `observable` | `4` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `mimxrt700_evk` | `pass` | `True` | `yes` | `yes` | `7586` | `7.586` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `mimxrt798s` | `pass` | `True` | `yes` | `yes` | `2783` | `2.783` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `miv` | `pass` | `True` | `yes` | `yes` | `1753` | `1.753` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `miv-board` | `pass` | `True` | `yes` | `yes` | `2671` | `2.671` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `miv-board-additional-uarts` | `pass` | `True` | `yes` | `yes` | `1551` | `1.551` | `True` | `observable` | `7` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `miv_rv32` | `pass` | `True` | `yes` | `yes` | `1520` | `1.52` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `mpc5567` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `5` | n/a | 3: `fec:fastethernetcontroller`, `emios:emios`, `fmpllSynsr:pythonperipheral` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no PowerPC e200z6 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `mpfs-icicle-kit` | `pass` | `True` | `yes` | `yes` | `804` | `0.804` | `True` | `observable` | `8` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `msp430f2619` | `pass` | `True` | `yes` | `yes` | `981` | `0.981` | `True` | `observable` | `7` | `uart_ordered`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `mspsim-msp430f149` | `pass` | `True` | `yes` | `yes` | `1358` | `1.358` | `True` | `observable` | `6` | `uart_ordered`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `mspsim-msp430f1611` | `pass` | `True` | `yes` | `yes` | `3923` | `3.923` | `True` | `observable` | `6` | `uart_ordered`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `mspsim-msp430f2617` | `pass` | `True` | `yes` | `yes` | `766` | `0.766` | `True` | `observable` | `7` | `uart_ordered`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `mspsim-msp430f5437` | `pass` | `True` | `yes` | `yes` | `186380` | `186.38` | `True` | `observable` | `7` | `uart_ordered`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `murax_vexriscv` | `pass` | `True` | `yes` | `yes` | `3037` | `3.037` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `murax_vexriscv_verilated_uart` | `pass` | `True` | `yes` | `yes` | `2455` | `2.455` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nrf52840` | `pass` | `True` | `yes` | `yes` | `1601` | `1.601` | `True` | `observable` | `51` | `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `nrf52840dk_nrf52840` | `pass` | `True` | `yes` | `yes` | `235918` | `235.918` | `True` | `observable` | `51` | `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `nucleo_f767zi` | `pass` | `True` | `yes` | `yes` | `2379` | `2.379` | `True` | `observable` | `53` | `uart_regex`, `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `nucleo_h753zi` | `pass` | `True` | `yes` | `yes` | `2273` | `2.273` | `True` | `observable` | `5` | `uart_regex`, `uart_contains`, `uart_ordered`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `nucleo_wba52cg` | `pass` | `True` | `yes` | `yes` | `4782` | `4.782` | `True` | `observable` | `4` | `uart_contains`, `uart_ordered`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `nuvoton_npcx9` | `pass` | `True` | `yes` | `yes` | `1630` | `1.63` | `True` | `observable` | `46` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nuvoton_npcx9m6fb_evb` | `pass` | `True` | `yes` | `yes` | `3009` | `3.009` | `True` | `observable` | `46` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nxp-frdm-imx8mplus` | `pass` | `True` | `yes` | `yes` | `765` | `0.765` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nxp-k6xf` | `pass` | `True` | `yes` | `yes` | `952` | `0.952` | `True` | `observable` | `7` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `nxp-mimx8ml` | `pass` | `True` | `yes` | `yes` | `960` | `0.96` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nxp-s32k388` | `pass` | `True` | `yes` | `yes` | `1538` | `1.538` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `nxp-s32k388evb` | `pass` | `True` | `yes` | `yes` | `1902` | `1.902` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `opentitan-earlgrey` | `pass` | `True` | `yes` | `yes` | `1218` | `1.218` | `True` | `observable` | `1` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `opentitan-earlgrey-cw310` | `pass` | `True` | `yes` | `yes` | `1559` | `1.559` | `True` | `observable` | `1` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `picosoc` | `pass` | `True` | `yes` | `yes` | `1717` | `1.717` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `polarfire-soc` | `pass` | `True` | `yes` | `yes` | `1227` | `1.227` | `True` | `observable` | `8` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `quark-c1000` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `7` | n/a | 4: `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `quark_c1000-cc2520` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `11` | n/a | 8: `led:led`, `button:button`, `radio:cc2520`, `lm74:ti_lm74`, `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ramn` | `pass` | `True` | `yes` | `yes` | `2963` | `2.963` | `True` | `observable` | `4` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `renesas-ck_ra6m5` | `pass` | `True` | `yes` | `yes` | `7609` | `7.609` | `True` | `observable` | `6` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-da14592` | `pass` | `True` | `yes` | `yes` | `1920` | `1.92` | `True` | `observable` | `3` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-ek_ra2e1` | `pass` | `True` | `yes` | `yes` | `1916` | `1.916` | `True` | `observable` | `4` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-ek_ra8m1` | `pass` | `True` | `yes` | `yes` | `1793` | `1.793` | `True` | `observable` | `9` | `uart_contains`, `memory_value`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa2e1a9` | `pass` | `True` | `yes` | `yes` | `2044` | `2.044` | `True` | `observable` | `3` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa2l1a` | `pass` | `True` | `yes` | `yes` | `1616` | `1.616` | `True` | `observable` | `3` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa4m1a` | `pass` | `True` | `yes` | `yes` | `5336` | `5.336` | `True` | `observable` | `6` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa6m5b` | `pass` | `True` | `yes` | `yes` | `1228` | `1.228` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa8m1a` | `pass` | `True` | `yes` | `yes` | `2685` | `2.685` | `True` | `observable` | `5` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-rz_t2m_rsk` | `pass` | `True` | `yes` | `yes` | `3808` | `3.808` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `renesas_rz_g2l` | `pass` | `True` | `yes` | `yes` | `3164` | `3.164` | `True` | `observable` | `5` | `uart_regex`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `renesas_rz_t2m` | `pass` | `True` | `yes` | `yes` | `4916` | `4.916` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `ri5cy` | `pass` | `True` | `yes` | `yes` | `2531` | `2.531` | `True` | `observable` | `1` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `riscv_verilated_uartlite` | `pass` | `True` | `yes` | `yes` | `1603` | `1.603` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `riscv_virt` | `pass` | `True` | `yes` | `yes` | `1120` | `1.12` | `True` | `observable` | `1` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `s32k118` | `pass` | `True` | `yes` | `yes` | `2855` | `2.855` | `True` | `observable` | `6` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam4s` | `pass` | `True` | `yes` | `yes` | `4448` | `4.448` | `True` | `observable` | `13` | `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam4s16c` | `pass` | `True` | `yes` | `yes` | `332072` | `332.072` | `True` | `observable` | `13` | `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam4s8b` | `pass` | `True` | `yes` | `yes` | `4491` | `4.491` | `True` | `observable` | `12` | `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam4s_xplained` | `pass` | `True` | `yes` | `yes` | `7209` | `7.209` | `True` | `observable` | `13` | `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam_e70` | `pass` | `True` | `yes` | `yes` | `4512` | `4.512` | `True` | `observable` | `19` | `uart_ordered`, `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sifive-fe310` | `pass` | `True` | `yes` | `yes` | `3191` | `3.191` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `sifive-fu540` | `pass` | `True` | `yes` | `yes` | `2299` | `2.299` | `True` | `observable` | `1` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sifive-fu740` | `pass` | `True` | `yes` | `yes` | `1032` | `1.032` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `simavr-at90usb162` | `pass` | `True` | `yes` | `yes` | `943` | `0.943` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128` | `pass` | `True` | `yes` | `yes` | `925` | `0.925` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1280` | `pass` | `True` | `yes` | `yes` | `1862` | `1.862` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1281` | `pass` | `True` | `yes` | `yes` | `2427` | `2.427` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1284p` | `pass` | `True` | `yes` | `yes` | `1402` | `1.402` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128rfa1` | `pass` | `True` | `yes` | `yes` | `1183` | `1.183` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128rfr2` | `pass` | `True` | `yes` | `yes` | `1417` | `1.417` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega16` | `pass` | `True` | `yes` | `yes` | `757` | `0.757` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega164` | `pass` | `True` | `yes` | `yes` | `764` | `0.764` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega168` | `pass` | `True` | `yes` | `yes` | `743` | `0.743` | `True` | `observable` | `4` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega169p` | `pass` | `True` | `yes` | `yes` | `777` | `0.777` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega16m1` | `pass` | `True` | `yes` | `yes` | `828` | `0.828` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega2560` | `pass` | `True` | `yes` | `yes` | `4600` | `4.6` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega32` | `pass` | `True` | `yes` | `yes` | `4803` | `4.803` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega324` | `pass` | `True` | `yes` | `yes` | `4454` | `4.454` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega328` | `pass` | `True` | `yes` | `yes` | `3299` | `3.299` | `True` | `observable` | `2` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega328pb` | `pass` | `True` | `yes` | `yes` | `911` | `0.911` | `True` | `observable` | `2` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega32u4` | `pass` | `True` | `yes` | `yes` | `1143` | `1.143` | `True` | `observable` | `1` | `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega48` | `pass` | `True` | `yes` | `yes` | `928` | `0.928` | `True` | `observable` | `4` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega644` | `pass` | `True` | `yes` | `yes` | `1006` | `1.006` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega64m1` | `pass` | `True` | `yes` | `yes` | `959` | `0.959` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega8` | `pass` | `True` | `yes` | `yes` | `2834` | `2.834` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega88` | `pass` | `True` | `yes` | `yes` | `1713` | `1.713` | `True` | `observable` | `4` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny13` | `pass` | `True` | `yes` | `yes` | `2217` | `2.217` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny1634` | `pass` | `True` | `yes` | `yes` | `1670` | `1.67` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny2313` | `pass` | `True` | `yes` | `yes` | `2088` | `2.088` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny2313a` | `pass` | `True` | `yes` | `yes` | `804` | `0.804` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny24` | `pass` | `True` | `yes` | `yes` | `771` | `0.771` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny25` | `pass` | `True` | `yes` | `yes` | `840` | `0.84` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny4313` | `pass` | `True` | `yes` | `yes` | `853` | `0.853` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny44` | `pass` | `True` | `yes` | `yes` | `781` | `0.781` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny45` | `pass` | `True` | `yes` | `yes` | `1386` | `1.386` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny84` | `pass` | `True` | `yes` | `yes` | `771` | `0.771` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny85` | `pass` | `True` | `yes` | `yes` | `821` | `0.821` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `sltb001a` | `pass` | `True` | `yes` | `yes` | `2790` | `2.79` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sltb004a` | `pass` | `True` | `yes` | `yes` | `1391` | `1.391` | `True` | `observable` | `12` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `slwstk6220a` | `pass` | `True` | `yes` | `yes` | `1539` | `1.539` | `True` | `observable` | `14` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `starfive-jh7100` | `pass` | `True` | `yes` | `yes` | `2109` | `2.109` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `stk3200` | `pass` | `True` | `yes` | `yes` | `1214` | `1.214` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `stk3600` | `pass` | `True` | `yes` | `yes` | `1490` | `1.49` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `stk3700` | `pass` | `True` | `yes` | `yes` | `2923` | `2.923` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `stk3800` | `pass` | `True` | `yes` | `yes` | `3757` | `3.757` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `stm32f042` | `pass` | `True` | `yes` | `yes` | `1872` | `1.872` | `True` | `observable` | `21` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f072` | `pass` | `True` | `yes` | `yes` | `1939` | `1.939` | `True` | `observable` | `29` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f072b_discovery` | `pass` | `True` | `yes` | `yes` | `1420` | `1.42` | `True` | `observable` | `29` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f103` | `pass` | `True` | `yes` | `yes` | `441263` | `441.263` | `True` | `observable` | `9` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `stm32f4` | `pass` | `True` | `yes` | `yes` | `1779` | `1.779` | `True` | `observable` | `7` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `stm32f401cdu6-blackpill` | `pass` | `True` | `yes` | `yes` | `238403` | `238.403` | `True` | `observable` | `29` | `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `stm32f412` | `pass` | `True` | `yes` | `yes` | `1239` | `1.239` | `True` | `observable` | `6` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f429` | `pass` | `True` | `yes` | `yes` | `1307` | `1.307` | `True` | `observable` | `11` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f4_discovery` | `pass` | `True` | `yes` | `yes` | `6416` | `6.416` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `stm32f4_discovery-kit` | `pass` | `True` | `yes` | `yes` | `2295` | `2.295` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `stm32f746` | `pass` | `True` | `yes` | `yes` | `6153` | `6.153` | `True` | `observable` | `49` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f777` | `pass` | `True` | `yes` | `yes` | `2469` | `2.469` | `True` | `observable` | `53` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f7_discovery-bb` | `pass` | `True` | `yes` | `yes` | `1567` | `1.567` | `True` | `observable` | `49` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h7` | `pass` | `True` | `yes` | `yes` | `3854` | `3.854` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h743` | `pass` | `True` | `yes` | `yes` | `5072` | `5.072` | `True` | `observable` | `8` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h747` | `pass` | `True` | `yes` | `yes` | `4431` | `4.431` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h747-disco` | `pass` | `True` | `yes` | `yes` | `3227` | `3.227` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h753` | `pass` | `True` | `yes` | `yes` | `2686` | `2.686` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32l071` | `pass` | `True` | `yes` | `yes` | `5083` | `5.083` | `True` | `observable` | `28` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32l072` | `pass` | `True` | `yes` | `yes` | `5917` | `5.917` | `True` | `observable` | `30` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32l151` | `pass` | `True` | `yes` | `yes` | `3780` | `3.78` | `True` | `observable` | `27` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32l552` | `pass` | `True` | `yes` | `yes` | `1450` | `1.45` | `True` | `observable` | `8` | `uart_regex`, `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `stm32w108` | `pass` | `True` | `yes` | `yes` | `600477` | `600.477` | `True` | `observable` | `3` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `stm32wba52` | `pass` | `True` | `yes` | `yes` | `547784` | `547.784` | `True` | `observable` | `4` | `uart_contains`, `uart_ordered`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `tegra2` | `pass` | `True` | `yes` | `yes` | `1079` | `1.079` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `tegra3` | `pass` | `True` | `yes` | `yes` | `1531` | `1.531` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `tock_veer_el2_sim` | `pass` | `True` | `yes` | `yes` | `1213` | `1.213` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `up_squared_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `3` | n/a | 2: `hpet:hpet`, `pci:pythonperipheral` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no x86-64 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ut32m0r500` | `pass` | `True` | `yes` | `yes` | `955` | `0.955` | `True` | `observable` | `7` | `uart_contains`, `memory_value`, `gpio_edges`, `fidelity_clean` | 0 | `n/a` | n/a | n/a |
| `vegaboard_ri5cy` | `pass` | `True` | `yes` | `yes` | `4718` | `4.718` | `True` | `observable` | `1` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `verilated_ibex` | `pass` | `True` | `yes` | `yes` | `1955` | `1.955` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `versatile` | `pass` | `True` | `yes` | `yes` | `1331` | `1.331` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `vexpress` | `pass` | `True` | `yes` | `yes` | `1904` | `1.904` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `vybrid` | `pass` | `True` | `yes` | `yes` | `1497` | `1.497` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `x86` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `2` | n/a | 1: `hpet:hpet` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (x86) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `xtensa-sample-controller` | `pass` | `True` | `yes` | `yes` | `5448` | `5.448` | `True` | `observable` | `0` | `memory_value` | 0 | `n/a` | n/a | n/a |
| `zedboard` | `pass` | `True` | `yes` | `yes` | `1997` | `1.997` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zolertia-firefly` | `pass` | `True` | `yes` | `yes` | `4562` | `4.562` | `True` | `observable` | `24` | `uart_contains`, `memory_value`, `gpio_edges`, `fidelity_clean` | 0 | `n/a` | n/a | n/a |
| `zynq-7000` | `pass` | `True` | `yes` | `yes` | `3593` | `3.593` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp` | `pass` | `True` | `yes` | `yes` | `851` | `0.851` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu102-revA` | `pass` | `True` | `yes` | `yes` | `4692` | `4.692` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu102-revB` | `pass` | `True` | `yes` | `yes` | `1621` | `1.621` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu104` | `pass` | `True` | `yes` | `yes` | `2078` | `2.078` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu106-vcu` | `pass` | `True` | `yes` | `yes` | `2162` | `2.162` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
