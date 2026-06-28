# firmware-kw41z-nxp

A real, unmodified-NXP-vendor-code firmware ELF for the NXP **MKW41Z4** (KW41Z,
Cortex-M0+). It boots through the genuine NXP MCUXpresso clock bring-up
(`SystemInit` → `BOARD_BootClockRUN` → `fsl_clock.c` FEE-mode helpers) and then
prints the ASCII banner `KW41Z_NXP_OK\n` over **LPUART0** using the real NXP
`fsl_lpuart.c` driver.

This is an emulator fixture. The whole point is that the vendor clock-bring-up
code — which spins on `MCG_S` / `RSIM_CONTROL` status bits — executes
**unmodified**.

## Build

```sh
./build.sh          # -> build/kw41z-nxp.elf
```

Toolchain: `arm-none-eabi-gcc` 13.2.
Flags: `-mcpu=cortex-m0plus -mthumb -mfloat-abi=soft -ffunction-sections
-fdata-sections -Os -g -DCPU_MKW41Z512VHT4`, linked `-nostartfiles
-Wl,--gc-sections -T linker.ld --specs=nano.specs --specs=nosys.specs`.

## File provenance

All files under `vendor/` are **verbatim, unmodified** NXP / ARM sources with
their original BSD-3-Clause / Apache-2.0 copyright headers preserved.

### Verbatim NXP (from `github.com/nxp-mcuxpresso/mcux-sdk`, branch `main`)

| file (in `vendor/`)   | mcux-sdk path                                   |
|-----------------------|-------------------------------------------------|
| `system_MKW41Z4.c/.h` | `devices/MKW41Z4/system_MKW41Z4.c/.h`           |
| `MKW41Z4.h`           | `devices/MKW41Z4/MKW41Z4.h`                      |
| `MKW41Z4_features.h`  | `devices/MKW41Z4/MKW41Z4_features.h`             |
| `fsl_device_registers.h` | `devices/MKW41Z4/fsl_device_registers.h`     |
| `fsl_clock.c/.h`      | `devices/MKW41Z4/drivers/fsl_clock.c/.h`         |
| `fsl_lpuart.c/.h`     | `drivers/lpuart/fsl_lpuart.c/.h`                 |
| `fsl_common.c/.h`     | `drivers/common/fsl_common.c/.h`                 |
| `fsl_common_arm.c/.h` | `drivers/common/fsl_common_arm.c/.h`             |
| `fsl_smc.c/.h`        | `drivers/smc/fsl_smc.c/.h`                       |
| `startup_MKW41Z4.S`   | NXP GNU startup, mirror `github.com/benemorius/mcux_kw41z` `MKW41Z4/gcc/startup_MKW41Z4.S` |

### Verbatim NXP board file

| file (in `vendor/`)      | source                                                                 |
|--------------------------|------------------------------------------------------------------------|
| `clock_config.c/.h`      | NXP FRDM-KW41Z board clock config (`BOARD_BootClockRUN` = FEE, 40 MHz). Mirror `github.com/0xdeadbeefnetwork/nxp-se050-middleware` `demos/ksdk/common/boards/frdmkw41z/clock_config.c/.h` |

### Verbatim ARM CMSIS (from `github.com/ARM-software/CMSIS_5`, branch `develop`, `CMSIS/Core/Include`)

`core_cm0plus.h`, `cmsis_gcc.h`, `cmsis_version.h`, `cmsis_compiler.h`,
`mpu_armv7.h`.

### Authored by us (NOT under test)

| file          | purpose                                                                |
|---------------|------------------------------------------------------------------------|
| `main.c`      | Calls `BOARD_BootClockRUN()` then drives LPUART0 (`fsl_lpuart.c`) to emit the banner. The only hand-written C. |
| `linker.ld`   | MKW41Z512 memory map; supplies the symbols the vendor startup needs.    |
| `build.sh`    | Compile + link recipe.                                                  |

The startup is assembled with `-D__STARTUP_CLEAR_BSS -D__START=main` so
`Reset_Handler` copies `.data`, zeroes `.bss`, calls `SystemInit`, and branches
straight to `main` (no newlib `_start`/`crt0`).

## Boot flow

```
Reset_Handler            (vendor startup_MKW41Z4.S)
  -> SystemInit          (vendor system_MKW41Z4.c)   sets VTOR, disables WDOG
  -> main                (main.c)
       -> BOARD_BootClockRUN          (vendor clock_config.c)
            -> BOARD_RfOscInit              enable RSIM RF osc, spin on RF_OSC_READY
            -> CLOCK_SetSimSafeDivs         (fsl_clock.c) SIM->CLKDIV1 safe divs
            -> CLOCK_InitOsc0              (fsl_clock.c) MCG->C2 EREFS, spin OSCINIT0
            -> CLOCK_BootToFeeMode/SetFeeMode (fsl_clock.c) spin IREFST, CLKST=FLL
            -> CLOCK_SetInternalRefClkConfig  (fsl_clock.c)
            -> CLOCK_SetSimConfig             (fsl_clock.c) SIM->CLKDIV1=0x00010000
            SystemCoreClock = 40 MHz
       -> CLOCK_SetLpuartClock(OSCERCLK)   SIM->SOPT2[LPUART0SRC]=0b10
       -> CLOCK_EnableClock(kCLOCK_PortC)  SIM->SCGC5
       -> PORTC->PCR[6]/[7] = ALT4         LPUART0 RX/TX mux
       -> LPUART_Init / LPUART_WriteBlocking (vendor fsl_lpuart.c)  "KW41Z_NXP_OK\n"
       -> while(1)
```

LPUART output uses the **real** `fsl_lpuart.c` driver (`LPUART_GetDefaultConfig`
+ `LPUART_Init` + `LPUART_WriteBlocking`); we pass `srcClock_Hz = 32_000_000`
(OSCERCLK from the 32 MHz RF crystal) so the baud divider is well-defined and
there is no div-by-zero. No fall-back to inline TX was needed.
