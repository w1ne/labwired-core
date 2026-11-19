# Onboarding Smoke Scoreboard

- descriptors_present: `245`
- targets_total: `245`
- run_attempted: `226`
- booted: `226/245`
- boot_and_smoke_passed: `209/245`
- not_run: `19`
- pass: `209`
- fail: `17`
- pass_loading_zero_answering_windows: `0/209`
- pass_observable_assertions: `209/209`
- pass_stop_reason_only: `0/209`
- pass_depth_unknown: `0/209`
- pass_claimed_peripherals_total: `1483`
- pass_claimed_peripherals_median: `3`
- median_elapsed_ms: `1691`
- median_elapsed_seconds: `1.691`
- threshold_seconds: `3600`
- threshold_met: `226/226`

> `booted` means a real run produced a result with at least one executed firmware instruction. A smoke pass proves only the assertions in that target's script, not full peripheral coverage. `not_run` means no firmware run was attempted for that descriptor. `Zero Windows` lists the peripherals in the chip the target's system actually loads that are allowlisted stubs (`crates/core/src/bus/known_stubs.rs`) and answer reads with zeros; a pass with zero windows did not exercise those blocks. `Smoke Depth` is `stop_reason_only` when the script asserts nothing but `expected_stop_reason` (the CPU stepped to its budget without faulting; no UART, memory or GPIO result is checked) and `observable` when it also checks a UART, memory, GPIO or fidelity result. `Claimed` is the number of peripheral windows the loaded chip profile declares and `Checks` the assertion kinds the smoke script uses; a smoke rarely exercises every claimed window, so a large `Claimed` next to a single `uart_contains` is a profile claim the smoke does not prove.

| Target | Status | Run Attempted | Booted | Smoke Passed | Elapsed (ms) | Elapsed (s) | Threshold Met | Smoke Depth | Claimed | Checks | Zero Windows | Failure Stage | Hint | Signature |
|---|---|---|---|---|---:|---:|---|---|---:|---|---|---|---|---|
| `A2_CV32E40P` | `pass` | `True` | `yes` | `yes` | `1805` | `1.805` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `a20` | `pass` | `True` | `yes` | `yes` | `270406` | `270.406` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `acrn_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `2` | n/a | 1: `hpet:hpet` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no x86-64 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `alif_ae822fa0e5597bs0` | `pass` | `True` | `yes` | `yes` | `767` | `0.767` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `ambiq-apollo4` | `pass` | `True` | `yes` | `yes` | `3581` | `3.581` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `andes_ae350_n25` | `pass` | `True` | `yes` | `yes` | `1400` | `1.4` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `arduino_101-shield` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `10` | n/a | 7: `led:led`, `button:button`, `lm74:ti_lm74`, `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `arduino_nano_33_ble` | `pass` | `True` | `yes` | `yes` | `3097` | `3.097` | `True` | `observable` | `50` | `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `arduino_uno_r4_minima` | `fail` | `True` | `yes` | `no` | `5277` | `5.277` | `True` | `observable` | `5` | `uart_contains`, `gpio_edges` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:26:29.970500Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "OK" }) (captured len=0) |
| `arty_litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `4095` | `4.095` | `True` | `observable` | `4` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `arvsom` | `pass` | `True` | `yes` | `yes` | `6431` | `6.431` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `at91rm9200` | `pass` | `True` | `yes` | `yes` | `368699` | `368.699` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `atsamd21j17d-aft` | `fail` | `True` | `yes` | `no` | `7635` | `7.635` | `True` | `observable` | `14` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:26:38.589984Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["samd21 sercom0 up", "portb dir=00000400 out=00000400 clr=00000000", "tc4 cc0=000003e7 adv=1 flags=11 nvic19=1 clea |
| `atsamd51g19a` | `pass` | `True` | `yes` | `yes` | `1851` | `1.851` | `True` | `observable` | `4` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `beaglev-fire` | `pass` | `True` | `yes` | `yes` | `1862` | `1.862` | `True` | `observable` | `8` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `beaglev_starlight` | `pass` | `True` | `yes` | `yes` | `2791` | `2.791` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4116a` | `pass` | `True` | `yes` | `yes` | `2124` | `2.124` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4117a` | `pass` | `True` | `yes` | `yes` | `3899` | `3.899` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4118a` | `pass` | `True` | `yes` | `yes` | `1913` | `1.913` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4120a` | `pass` | `True` | `yes` | `yes` | `1854` | `1.854` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4121a` | `pass` | `True` | `yes` | `yes` | `1333` | `1.333` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `brd4162a` | `pass` | `True` | `yes` | `yes` | `4156` | `4.156` | `True` | `observable` | `12` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `brd4186c` | `pass` | `True` | `yes` | `yes` | `2518` | `2.518` | `True` | `observable` | `6` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `brd4402a` | `pass` | `True` | `yes` | `yes` | `1514` | `1.514` | `True` | `observable` | `5` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `cc2538` | `pass` | `True` | `yes` | `yes` | `2935` | `2.935` | `True` | `observable` | `24` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `colibri-vf61` | `pass` | `True` | `yes` | `yes` | `1124` | `1.124` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `core-v-mcu` | `pass` | `True` | `yes` | `yes` | `1442` | `1.442` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv2` | `pass` | `True` | `yes` | `yes` | `1441` | `1.441` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv3` | `pass` | `True` | `yes` | `yes` | `1931` | `1.931` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-a53-gicv3_smp` | `pass` | `True` | `yes` | `yes` | `1183` | `1.183` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-a78` | `pass` | `True` | `yes` | `yes` | `755` | `0.755` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-r52` | `pass` | `True` | `yes` | `yes` | `652` | `0.652` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-r52_smp` | `pass` | `True` | `yes` | `yes` | `632` | `0.632` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex-r52_smp_4` | `pass` | `True` | `yes` | `yes` | `677` | `0.677` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex_a53_console` | `pass` | `True` | `yes` | `yes` | `1369` | `1.369` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `cortex_a53_virtio` | `pass` | `True` | `yes` | `yes` | `761` | `0.761` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `crosslink-nx-evn` | `pass` | `True` | `yes` | `yes` | `1869` | `1.869` | `True` | `observable` | `3` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `efm32g210` | `pass` | `True` | `yes` | `yes` | `7972` | `7.972` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32g222` | `pass` | `True` | `yes` | `yes` | `6969` | `6.969` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32g232` | `pass` | `True` | `yes` | `yes` | `3638` | `3.638` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32g842` | `pass` | `True` | `yes` | `yes` | `5456` | `5.456` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32g890` | `pass` | `True` | `yes` | `yes` | `2434` | `2.434` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32gg942` | `pass` | `True` | `yes` | `yes` | `2515` | `2.515` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32gg995` | `pass` | `True` | `yes` | `yes` | `4415` | `4.415` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32hg350` | `pass` | `True` | `yes` | `yes` | `1785` | `1.785` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32jg1` | `pass` | `True` | `yes` | `yes` | `3512` | `3.512` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32jg12` | `pass` | `True` | `yes` | `yes` | `3539` | `3.539` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32lg942` | `pass` | `True` | `yes` | `yes` | `1691` | `1.691` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32lg995` | `pass` | `True` | `yes` | `yes` | `3527` | `3.527` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32pg1` | `pass` | `True` | `yes` | `yes` | `1348` | `1.348` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32pg12` | `pass` | `True` | `yes` | `yes` | `1281` | `1.281` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32tg840` | `pass` | `True` | `yes` | `yes` | `1327` | `1.327` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32wg995` | `pass` | `True` | `yes` | `yes` | `1442` | `1.442` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efm32zg222` | `pass` | `True` | `yes` | `yes` | `1524` | `1.524` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `efr32mg1` | `fail` | `True` | `yes` | `no` | `2926` | `2.926` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:06:32.371498Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["gpio pf4/pf5 pushpull set=1 tgl=1 clr=1 wo=1 ok", "gpio reserved 0x10/0x14 no_set=1 no_clr=1 read0=1 ok", "gpio pa |
| `efr32mg12` | `fail` | `True` | `yes` | `no` | `1121` | `1.121` | `True` | `observable` | `12` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:06:35.904605Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["gpio pf4/pf5 pushpull set=1 tgl=1 clr=1 wo=1 ok", "gpio reserved 0x10/0x14 no_set=1 no_clr=1 read0=1 ok", "gpio pa |
| `efr32mg13` | `fail` | `True` | `yes` | `no` | `3834` | `3.834` | `True` | `observable` | `11` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:06:43.188594Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["gpio pf4/pf5 pushpull set=1 tgl=1 clr=1 wo=1 ok", "gpio reserved 0x10/0x14 no_set=1 no_clr=1 read0=1 ok", "gpio pa |
| `efr32mg24` | `pass` | `True` | `yes` | `yes` | `1318` | `1.318` | `True` | `observable` | `6` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `efr32mg26` | `pass` | `True` | `yes` | `yes` | `2056` | `2.056` | `True` | `observable` | `4` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `efr32xg22` | `pass` | `True` | `yes` | `yes` | `1166` | `1.166` | `True` | `observable` | `5` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `egis_et171` | `pass` | `True` | `yes` | `yes` | `1418` | `1.418` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `eos-s3` | `pass` | `True` | `yes` | `yes` | `1507` | `1.507` | `True` | `observable` | `5` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `eos-s3-qomu` | `pass` | `True` | `yes` | `yes` | `1094` | `1.094` | `True` | `observable` | `5` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `eos-s3-quickfeather` | `pass` | `True` | `yes` | `yes` | `1193` | `1.193` | `True` | `observable` | `5` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `ezr32hg320` | `fail` | `True` | `yes` | `no` | `8072` | `8.072` | `True` | `observable` | `10` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:07:14.244137Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["gpio pf4/pf5 pushpull set=1 tgl=1 clr=1 wo=1 ok", "gpio pa0 disabled_din0=1 pullup=1 pulldown=1 ok", "gpio pa1 wir |
| `ezr32lg330` | `fail` | `True` | `yes` | `no` | `8836` | `8.836` | `True` | `observable` | `14` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:07:25.061065Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["uart1 status=40 ok", "usart1 status=40 ok", "usart2 status=40 ok", "EZR32 330 RENODE SMOKE", "gpio pe2/pe3 pushpul |
| `ezr32wg330` | `fail` | `True` | `yes` | `no` | `6785` | `6.785` | `True` | `observable` | `14` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:07:34.097348Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["uart1 status=40 ok", "usart1 status=40 ok", "usart2 status=40 ok", "EZR32 330 RENODE SMOKE", "gpio pe2/pe3 pushpul |
| `focaltech_ft9001` | `pass` | `True` | `yes` | `yes` | `1103` | `1.103` | `True` | `observable` | `5` | `uart_regex`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `fomu` | `pass` | `True` | `yes` | `yes` | `1172` | `1.172` | `True` | `observable` | `1` | `memory_value` | 0 | `n/a` | n/a | n/a |
| `fsl_lx2160ardb` | `pass` | `True` | `yes` | `yes` | `673` | `0.673` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `gr712rc` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `16` | n/a | 8: `ftmctrl:gaisler_faulttolerantmemorycontroller`, `gptimer:stub`, `grtimer:stub`, `greth:gaislereth`, `phy:ethernetphysicallayer`, `ahbInfo:gaislerahbplugandplayinfo`, `apb1Controller:gaislerapbcontroller`, `apb2Controller:gaislerapbcontroller` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `gr716` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `9` | n/a | 1: `timer0:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `gr716-devboard` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `13` | n/a | 5: `switch0:button`, `switch1:button`, `switch2:button`, `switch3:button`, `timer0:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3FT) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ice40up5k-mdp-evn` | `pass` | `True` | `yes` | `yes` | `958` | `0.958` | `True` | `observable` | `1` | `memory_value` | 0 | `n/a` | n/a | n/a |
| `imxrt1064` | `pass` | `True` | `yes` | `yes` | `1591` | `1.591` | `True` | `observable` | `5` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `imxrt500` | `pass` | `True` | `yes` | `yes` | `1711` | `1.711` | `True` | `observable` | `6` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `kendryte_k210` | `pass` | `True` | `yes` | `yes` | `2409` | `2.409` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `leon3` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `9` | n/a | 7: `ahbInfo:gaislerahbplugandplayinfo`, `apbController:gaislerapbcontroller`, `esaMemController:esamemorycontroller`, `timer:stub`, `eth:gaislereth`, `phy:ethernetphysicallayer`, `led:led` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no SPARC V8 (LEON3) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `litex_common` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `1` | n/a | 1: `phy:ethernetphysicallayer` | `not_configured` | Measure the concrete LiteX platforms that include it instead. | Shared LiteX include with no CPU of its own; it is not a runnable platform. |
| `litex_ibex` | `pass` | `True` | `yes` | `yes` | `1826` | `1.826` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_linux_vexriscv_sdcard` | `pass` | `True` | `yes` | `yes` | `2033` | `2.033` | `True` | `observable` | `1` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `2` | n/a | 1: `timer0:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no OpenPOWER (Microwatt) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `litex_minerva` | `pass` | `True` | `yes` | `yes` | `5843` | `5.843` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_nexys_video_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `3740` | `3.74` | `True` | `observable` | `2` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_picorv32` | `pass` | `True` | `yes` | `yes` | `3505` | `3.505` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_tock` | `pass` | `True` | `yes` | `yes` | `1692` | `1.692` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv` | `pass` | `True` | `yes` | `yes` | `2589` | `2.589` | `True` | `observable` | `3` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_linux` | `pass` | `True` | `yes` | `yes` | `2719` | `2.719` | `True` | `observable` | `2` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_micropython` | `pass` | `True` | `yes` | `yes` | `4249` | `4.249` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_smp` | `pass` | `True` | `yes` | `yes` | `1921` | `1.921` | `True` | `observable` | `1` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_tftp` | `pass` | `True` | `yes` | `yes` | `3349` | `3.349` | `True` | `observable` | `2` | `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_cfu` | `pass` | `True` | `yes` | `yes` | `2931` | `2.931` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_verilated_liteuart` | `pass` | `True` | `yes` | `yes` | `3348` | `3.348` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_vexriscv_zephyr` | `pass` | `True` | `yes` | `yes` | `2548` | `2.548` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `litex_zephyr_vexriscv_i2s` | `pass` | `True` | `yes` | `yes` | `2358` | `2.358` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `lpc2294` | `pass` | `True` | `yes` | `yes` | `245022` | `245.022` | `True` | `observable` | `11` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `mars_zx3` | `pass` | `True` | `yes` | `yes` | `1589` | `1.589` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `max32652` | `pass` | `True` | `yes` | `yes` | `2817` | `2.817` | `True` | `observable` | `7` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `max32652-evkit` | `pass` | `True` | `yes` | `yes` | `1425` | `1.425` | `True` | `observable` | `7` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `microwatt` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `1` | n/a | 1: `uart:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no OpenPOWER (Microwatt) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `mimxrt1064_evk` | `pass` | `True` | `yes` | `yes` | `1904` | `1.904` | `True` | `observable` | `4` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `mimxrt700_evk` | `pass` | `True` | `yes` | `yes` | `3086` | `3.086` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `mimxrt798s` | `pass` | `True` | `yes` | `yes` | `2855` | `2.855` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `miv` | `pass` | `True` | `yes` | `yes` | `2881` | `2.881` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `miv-board` | `pass` | `True` | `yes` | `yes` | `1368` | `1.368` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `miv-board-additional-uarts` | `pass` | `True` | `yes` | `yes` | `2415` | `2.415` | `True` | `observable` | `7` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `miv_rv32` | `pass` | `True` | `yes` | `yes` | `1392` | `1.392` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `mpc5567` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `5` | n/a | 3: `fec:fastethernetcontroller`, `emios:emios`, `fmpllSynsr:pythonperipheral` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no PowerPC e200z6 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `mpfs-icicle-kit` | `pass` | `True` | `yes` | `yes` | `849` | `0.849` | `True` | `observable` | `8` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `msp430f2619` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `4` | n/a | 4: `boot:arraymemory`, `ram_extended:arraymemory`, `code:arraymemory`, `info_mem:arraymemory` | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `mspsim-msp430f149` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `0` | n/a | 0 | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `mspsim-msp430f1611` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `0` | n/a | 0 | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `mspsim-msp430f2617` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `0` | n/a | 0 | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `mspsim-msp430f5437` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `0` | n/a | 0 | `not_configured` | Implement a real MSP430 CPU core and ELF architecture support before adding a smoke; do not substitute a peripheral stub. | MSP430 CPU execution and MSP430 ELF architecture support are not implemented; no firmware run was attempted. A real CPU core is required before this target can enter executable coverage. |
| `murax_vexriscv` | `pass` | `True` | `yes` | `yes` | `2725` | `2.725` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `murax_vexriscv_verilated_uart` | `pass` | `True` | `yes` | `yes` | `1222` | `1.222` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nrf52840` | `pass` | `True` | `yes` | `yes` | `3420` | `3.42` | `True` | `observable` | `50` | `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `nrf52840dk_nrf52840` | `pass` | `True` | `yes` | `yes` | `1821` | `1.821` | `True` | `observable` | `50` | `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `nucleo_f767zi` | `pass` | `True` | `yes` | `yes` | `2912` | `2.912` | `True` | `observable` | `53` | `uart_regex`, `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `nucleo_h753zi` | `pass` | `True` | `yes` | `yes` | `2940` | `2.94` | `True` | `observable` | `5` | `uart_regex`, `uart_contains`, `uart_ordered`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `nucleo_wba52cg` | `pass` | `True` | `yes` | `yes` | `2441` | `2.441` | `True` | `observable` | `4` | `uart_contains`, `uart_ordered`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `nuvoton_npcx9` | `pass` | `True` | `yes` | `yes` | `1475` | `1.475` | `True` | `observable` | `46` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nuvoton_npcx9m6fb_evb` | `pass` | `True` | `yes` | `yes` | `1581` | `1.581` | `True` | `observable` | `46` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nxp-frdm-imx8mplus` | `pass` | `True` | `yes` | `yes` | `767` | `0.767` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nxp-k6xf` | `pass` | `True` | `yes` | `yes` | `924` | `0.924` | `True` | `observable` | `7` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `nxp-mimx8ml` | `pass` | `True` | `yes` | `yes` | `816` | `0.816` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `nxp-s32k388` | `pass` | `True` | `yes` | `yes` | `785` | `0.785` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `nxp-s32k388evb` | `pass` | `True` | `yes` | `yes` | `886` | `0.886` | `True` | `observable` | `5` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `opentitan-earlgrey` | `pass` | `True` | `yes` | `yes` | `1784` | `1.784` | `True` | `observable` | `1` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `opentitan-earlgrey-cw310` | `pass` | `True` | `yes` | `yes` | `1036` | `1.036` | `True` | `observable` | `1` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `picosoc` | `pass` | `True` | `yes` | `yes` | `1239` | `1.239` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `polarfire-soc` | `pass` | `True` | `yes` | `yes` | `819` | `0.819` | `True` | `observable` | `8` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `quark-c1000` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `7` | n/a | 4: `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `quark_c1000-cc2520` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `11` | n/a | 8: `led:led`, `button:button`, `radio:cc2520`, `lm74:ti_lm74`, `spi0:stub`, `spi1:stub`, `pwm:stub`, `scss:stub` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (Intel Quark) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ramn` | `pass` | `True` | `yes` | `yes` | `1473` | `1.473` | `True` | `observable` | `4` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `renesas-ck_ra6m5` | `pass` | `True` | `yes` | `yes` | `5791` | `5.791` | `True` | `observable` | `6` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-da14592` | `pass` | `True` | `yes` | `yes` | `1333` | `1.333` | `True` | `observable` | `3` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-ek_ra2e1` | `pass` | `True` | `yes` | `yes` | `1281` | `1.281` | `True` | `observable` | `4` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-ek_ra8m1` | `pass` | `True` | `yes` | `yes` | `1537` | `1.537` | `True` | `observable` | `9` | `uart_contains`, `memory_value`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa2e1a9` | `pass` | `True` | `yes` | `yes` | `2329` | `2.329` | `True` | `observable` | `3` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa2l1a` | `pass` | `True` | `yes` | `yes` | `1385` | `1.385` | `True` | `observable` | `3` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa4m1a` | `fail` | `True` | `yes` | `no` | `7973` | `7.973` | `True` | `observable` | `5` | `uart_contains`, `gpio_edges` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:12:10.788644Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "OK" }) (captured len=0) |
| `renesas-r7fa6m5b` | `pass` | `True` | `yes` | `yes` | `981` | `0.981` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `renesas-r7fa8m1a` | `pass` | `True` | `yes` | `yes` | `1637` | `1.637` | `True` | `observable` | `5` | `memory_value`, `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `renesas-rz_t2m_rsk` | `pass` | `True` | `yes` | `yes` | `676` | `0.676` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `renesas_rz_g2l` | `pass` | `True` | `yes` | `yes` | `1070` | `1.07` | `True` | `observable` | `5` | `uart_regex`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `renesas_rz_t2m` | `pass` | `True` | `yes` | `yes` | `181038` | `181.038` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `ri5cy` | `pass` | `True` | `yes` | `yes` | `2993` | `2.993` | `True` | `observable` | `1` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `riscv_verilated_uartlite` | `pass` | `True` | `yes` | `yes` | `6885` | `6.885` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `riscv_virt` | `pass` | `True` | `yes` | `yes` | `1684` | `1.684` | `True` | `observable` | `1` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `s32k118` | `pass` | `True` | `yes` | `yes` | `4767` | `4.767` | `True` | `observable` | `6` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam4s` | `pass` | `True` | `yes` | `yes` | `1720` | `1.72` | `True` | `observable` | `13` | `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam4s16c` | `pass` | `True` | `yes` | `yes` | `2616` | `2.616` | `True` | `observable` | `13` | `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam4s8b` | `pass` | `True` | `yes` | `yes` | `2361` | `2.361` | `True` | `observable` | `12` | `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam4s_xplained` | `pass` | `True` | `yes` | `yes` | `5486` | `5.486` | `True` | `observable` | `13` | `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sam_e70` | `pass` | `True` | `yes` | `yes` | `3888` | `3.888` | `True` | `observable` | `19` | `uart_ordered`, `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sifive-fe310` | `pass` | `True` | `yes` | `yes` | `2342` | `2.342` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `sifive-fu540` | `pass` | `True` | `yes` | `yes` | `2604` | `2.604` | `True` | `observable` | `1` | `uart_contains`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `sifive-fu740` | `pass` | `True` | `yes` | `yes` | `2223` | `2.223` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `simavr-at90usb162` | `pass` | `True` | `yes` | `yes` | `1425` | `1.425` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128` | `pass` | `True` | `yes` | `yes` | `1210` | `1.21` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1280` | `pass` | `True` | `yes` | `yes` | `1004` | `1.004` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1281` | `pass` | `True` | `yes` | `yes` | `1070` | `1.07` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega1284p` | `pass` | `True` | `yes` | `yes` | `943` | `0.943` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128rfa1` | `pass` | `True` | `yes` | `yes` | `916` | `0.916` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega128rfr2` | `pass` | `True` | `yes` | `yes` | `1049` | `1.049` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega16` | `pass` | `True` | `yes` | `yes` | `1368` | `1.368` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega164` | `pass` | `True` | `yes` | `yes` | `852` | `0.852` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega168` | `pass` | `True` | `yes` | `yes` | `803` | `0.803` | `True` | `observable` | `4` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega169p` | `pass` | `True` | `yes` | `yes` | `857` | `0.857` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega16m1` | `pass` | `True` | `yes` | `yes` | `689` | `0.689` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega2560` | `pass` | `True` | `yes` | `yes` | `2160` | `2.16` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega32` | `pass` | `True` | `yes` | `yes` | `2549` | `2.549` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega324` | `pass` | `True` | `yes` | `yes` | `1738` | `1.738` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega328` | `pass` | `True` | `yes` | `yes` | `909` | `0.909` | `True` | `observable` | `2` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega328pb` | `pass` | `True` | `yes` | `yes` | `1097` | `1.097` | `True` | `observable` | `2` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega32u4` | `pass` | `True` | `yes` | `yes` | `4190` | `4.19` | `True` | `observable` | `1` | `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega48` | `pass` | `True` | `yes` | `yes` | `775` | `0.775` | `True` | `observable` | `4` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega644` | `pass` | `True` | `yes` | `yes` | `783` | `0.783` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega64m1` | `pass` | `True` | `yes` | `yes` | `938` | `0.938` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega8` | `pass` | `True` | `yes` | `yes` | `713` | `0.713` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-atmega88` | `pass` | `True` | `yes` | `yes` | `823` | `0.823` | `True` | `observable` | `4` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny13` | `pass` | `True` | `yes` | `yes` | `746` | `0.746` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny1634` | `pass` | `True` | `yes` | `yes` | `757` | `0.757` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny2313` | `pass` | `True` | `yes` | `yes` | `761` | `0.761` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny2313a` | `pass` | `True` | `yes` | `yes` | `750` | `0.75` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny24` | `pass` | `True` | `yes` | `yes` | `703` | `0.703` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny25` | `pass` | `True` | `yes` | `yes` | `3377` | `3.377` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny4313` | `pass` | `True` | `yes` | `yes` | `3577` | `3.577` | `True` | `observable` | `2` | `uart_contains`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny44` | `pass` | `True` | `yes` | `yes` | `3655` | `3.655` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny45` | `pass` | `True` | `yes` | `yes` | `3128` | `3.128` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny84` | `pass` | `True` | `yes` | `yes` | `741` | `0.741` | `True` | `observable` | `2` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `simavr-attiny85` | `pass` | `True` | `yes` | `yes` | `782` | `0.782` | `True` | `observable` | `1` | `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `sltb001a` | `fail` | `True` | `yes` | `no` | `1600` | `1.6` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:15:49.076876Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["gpio pf4/pf5 pushpull set=1 tgl=1 clr=1 wo=1 ok", "gpio reserved 0x10/0x14 no_set=1 no_clr=1 read0=1 ok", "gpio pa |
| `sltb004a` | `fail` | `True` | `yes` | `no` | `1050` | `1.05` | `True` | `observable` | `12` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:15:52.261938Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["gpio pf4/pf5 pushpull set=1 tgl=1 clr=1 wo=1 ok", "gpio reserved 0x10/0x14 no_set=1 no_clr=1 read0=1 ok", "gpio pa |
| `slwstk6220a` | `fail` | `True` | `yes` | `no` | `8345` | `8.345` | `True` | `observable` | `14` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:15:56.928095Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["uart1 status=40 ok", "usart1 status=40 ok", "usart2 status=40 ok", "SLWSTK6220A RENODE SMOKE", "gpio pe2/pe3 pushp |
| `starfive-jh7100` | `pass` | `True` | `yes` | `yes` | `1028` | `1.028` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `stk3200` | `fail` | `True` | `yes` | `no` | `7388` | `7.388` | `True` | `observable` | `9` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:16:09.872396Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["EFM32ZG222 RENODE SMOKE", "gpio pc10/pc11 pushpull set=1 tgl=1 clr=1 wo=1 ok", "gpio pa0 disabled_din0=1 pullup=1  |
| `stk3600` | `fail` | `True` | `yes` | `no` | `7223` | `7.223` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:16:19.603655Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["uart0 status=40 ok", "uart1 status=40 ok", "usart1 status=40 ok", "usart2 status=40 ok", "EFM32LG995 RENODE SMOKE" |
| `stk3700` | `fail` | `True` | `yes` | `no` | `7449` | `7.449` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:16:29.218340Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["uart0 status=40 ok", "uart1 status=40 ok", "usart1 status=40 ok", "usart2 status=40 ok", "EFM32GG995 RENODE SMOKE" |
| `stk3800` | `fail` | `True` | `yes` | `no` | `12202` | `12.202` | `True` | `observable` | `15` | `uart_contains`, `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:16:40.975013Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["uart1 status=40 ok", "usart0 status=40 ok", "usart1 status=40 ok", "usart2 status=40 ok", "EFM32WG995 RENODE SMOKE |
| `stm32f042` | `pass` | `True` | `yes` | `yes` | `4203` | `4.203` | `True` | `observable` | `21` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f072` | `pass` | `True` | `yes` | `yes` | `1961` | `1.961` | `True` | `observable` | `29` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f072b_discovery` | `pass` | `True` | `yes` | `yes` | `2139` | `2.139` | `True` | `observable` | `29` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f103` | `pass` | `True` | `yes` | `yes` | `1839` | `1.839` | `True` | `observable` | `9` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `stm32f4` | `pass` | `True` | `yes` | `yes` | `2365` | `2.365` | `True` | `observable` | `7` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `stm32f401cdu6-blackpill` | `pass` | `True` | `yes` | `yes` | `1414` | `1.414` | `True` | `observable` | `29` | `uart_ordered`, `gpio_edges`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `stm32f412` | `pass` | `True` | `yes` | `yes` | `1325` | `1.325` | `True` | `observable` | `6` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f429` | `pass` | `True` | `yes` | `yes` | `2464` | `2.464` | `True` | `observable` | `11` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f4_discovery` | `pass` | `True` | `yes` | `yes` | `1770` | `1.77` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `stm32f4_discovery-kit` | `pass` | `True` | `yes` | `yes` | `2330` | `2.33` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `stm32f746` | `pass` | `True` | `yes` | `yes` | `1630` | `1.63` | `True` | `observable` | `49` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f777` | `pass` | `True` | `yes` | `yes` | `3725` | `3.725` | `True` | `observable` | `53` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32f7_discovery-bb` | `pass` | `True` | `yes` | `yes` | `1294` | `1.294` | `True` | `observable` | `49` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h7` | `pass` | `True` | `yes` | `yes` | `1844` | `1.844` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h743` | `pass` | `True` | `yes` | `yes` | `2478` | `2.478` | `True` | `observable` | `8` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h747` | `pass` | `True` | `yes` | `yes` | `1615` | `1.615` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h747-disco` | `pass` | `True` | `yes` | `yes` | `1615` | `1.615` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32h753` | `pass` | `True` | `yes` | `yes` | `1624` | `1.624` | `True` | `observable` | `7` | `uart_contains`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32l071` | `pass` | `True` | `yes` | `yes` | `1374` | `1.374` | `True` | `observable` | `28` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32l072` | `pass` | `True` | `yes` | `yes` | `1590` | `1.59` | `True` | `observable` | `30` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32l151` | `pass` | `True` | `yes` | `yes` | `1523` | `1.523` | `True` | `observable` | `27` | `uart_contains`, `uart_ordered`, `memory_value`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `stm32l552` | `pass` | `True` | `yes` | `yes` | `986` | `0.986` | `True` | `observable` | `8` | `uart_regex`, `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `stm32w108` | `pass` | `True` | `yes` | `yes` | `1387` | `1.387` | `True` | `observable` | `3` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `stm32wba52` | `pass` | `True` | `yes` | `yes` | `1566` | `1.566` | `True` | `observable` | `4` | `uart_contains`, `uart_ordered`, `gpio_edges` | 0 | `n/a` | n/a | n/a |
| `tegra2` | `pass` | `True` | `yes` | `yes` | `1154` | `1.154` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `tegra3` | `pass` | `True` | `yes` | `yes` | `1043` | `1.043` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `tock_veer_el2_sim` | `pass` | `True` | `yes` | `yes` | `1264` | `1.264` | `True` | `observable` | `1` | `memory_value`, `uart_contains` | 0 | `n/a` | n/a | n/a |
| `up_squared_x86_64` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `3` | n/a | 2: `hpet:hpet`, `pci:pythonperipheral` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no x86-64 CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `ut32m0r500` | `fail` | `True` | `yes` | `no` | `3330` | `3.33` | `True` | `observable` | `7` | `uart_contains`, `memory_value`, `gpio_edges`, `fidelity_clean` | 0 | `run_smoke` | Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output. | 2026-11-19T16:18:45.117442Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "UT32 checks PPPPPPPPPPPP DONE" }) (captured len=80) |
| `vegaboard_ri5cy` | `pass` | `True` | `yes` | `yes` | `1399` | `1.399` | `True` | `observable` | `1` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `verilated_ibex` | `pass` | `True` | `yes` | `yes` | `958` | `0.958` | `True` | `observable` | `2` | `uart_contains`, `uart_ordered` | 0 | `n/a` | n/a | n/a |
| `versatile` | `pass` | `True` | `yes` | `yes` | `154313` | `154.313` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `vexpress` | `pass` | `True` | `yes` | `yes` | `1062` | `1.062` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `vybrid` | `pass` | `True` | `yes` | `yes` | `1391` | `1.391` | `True` | `observable` | `8` | `uart_contains`, `uart_ordered`, `memory_value` | 0 | `n/a` | n/a | n/a |
| `x86` | `not_run` | `False` | `n/a` | `n/a` | `0` | `0.0` | `n/a` | `n/a` | `2` | n/a | 1: `hpet:hpet` | `not_configured` | Add a real CPU core for this ISA before adding a smoke; never run it as a Cortex-M. | The engine has no IA-32 (x86) CPU core; the chip declares `arch: unknown` and is refused. No firmware run was attempted. |
| `xtensa-sample-controller` | `pass` | `True` | `yes` | `yes` | `779` | `0.779` | `True` | `observable` | `0` | `memory_value` | 0 | `n/a` | n/a | n/a |
| `zedboard` | `pass` | `True` | `yes` | `yes` | `1906` | `1.906` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zolertia-firefly` | `pass` | `True` | `yes` | `yes` | `1818` | `1.818` | `True` | `observable` | `24` | `uart_contains`, `memory_value`, `gpio_edges`, `fidelity_clean` | 0 | `n/a` | n/a | n/a |
| `zynq-7000` | `pass` | `True` | `yes` | `yes` | `138530` | `138.53` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp` | `pass` | `True` | `yes` | `yes` | `727` | `0.727` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu102-revA` | `pass` | `True` | `yes` | `yes` | `733` | `0.733` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu102-revB` | `pass` | `True` | `yes` | `yes` | `658` | `0.658` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu104` | `pass` | `True` | `yes` | `yes` | `698` | `0.698` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
| `zynqmp-zcu106-vcu` | `pass` | `True` | `yes` | `yes` | `985` | `0.985` | `True` | `observable` | `1` | `uart_contains` | 0 | `n/a` | n/a | n/a |
