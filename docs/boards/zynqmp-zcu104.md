# Validation Report: zynqmp-zcu104

**Architecture:** Quad ARM Cortex-A53.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `phy3` | `0x0000000c` | `ethernetphysicallayer` |
| `i2c_mux` | `0x00000074` | `pca9548` |
| `tca6416_u97` | `0x00000020` | `tca6416` |
| `apu0Timer` | `0x0000000f` | `arm_generictimer` |
| `apu1Timer` | `0x0000000f` | `arm_generictimer` |
| `apu2Timer` | `0x0000000f` | `arm_generictimer` |
| `apu3Timer` | `0x0000000f` | `arm_generictimer` |
| `uart0` | `0xff000000` | `cadence_uart` |
| `uart1` | `0xff010000` | `cadence_uart` |
| `i2c0` | `0xff020000` | `cadence_i2c` |
| `i2c1` | `0xff030000` | `cadence_i2c` |
| `ttc0` | `0xff110000` | `cadence_ttc` |
| `ttc1` | `0xff120000` | `cadence_ttc` |
| `ttc2` | `0xff130000` | `cadence_ttc` |
| `ttc3` | `0xff140000` | `cadence_ttc` |
| `gem0` | `0xff0b0000` | `cadencegem` |
| `gem1` | `0xff0c0000` | `cadencegem` |
| `gem2` | `0xff0d0000` | `cadencegem` |
| `gem3` | `0xff0e0000` | `cadencegem` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `gpio` | `0xff0a0000` | `xilinxgpiops` |
| `ipi` | `0xff300000` | `zynqmp_ipi` |
| `rtc` | `0xffa60000` | `zynqmp_rtc` |
| `platformManagementUnit` | `0x0000000a` | `zynqmp_platformmanagementunit` |
| `L1_PLL_STATUS_READ_1_SERDES_REGISTER` | `0xfd4063e4` | `arraymemory` |
| `L2_PLL_STATUS_READ_1_SERDES_REGISTER` | `0xfd40a3e4` | `arraymemory` |
| `L3_PLL_STATUS_READ_1_SERDES_REGISTER` | `0xfd40e3e4` | `arraymemory` |
| `PS_CTRL_STATUS_AMS_REGISTER` | `0xffa50040` | `pythonperipheral` |
| `qspi` | `0xff0f0000` | `zynqmp_gqspi` |

**Total Peripherals:** 29