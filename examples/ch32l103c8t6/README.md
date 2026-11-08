# WCH CH32L103C8T6 (QingKe V4C) — ch32fun CH32L103 examples

The CH32L103 is WCH's low-power PDUSB part: a 96 MHz QingKe V4C RISC-V core
(RV32IMAC), 62 KB flash, 20 KB SRAM, USB full-speed and USB PD/Type-C PHYs,
4 USARTs, 2 I2C, 2 SPI, CAN, a low-power timer, OPA/comparators and TouchKey.
The LQFP48 CH32L103C8T6 is $0.78 at LCSC. Live availability:
[evidence/vendor-status.md](evidence/vendor-status.md).

Demand: the WCH (Tier C) vendor bet in `DEMAND-ORDERED-TARGETS.md`, one more
CH32 family. It is not a survey top-5 MCU. It was picked because it is the
newest family WCH's Arduino core ships that the engine did not reach yet.
That Arduino path turned out to be broken upstream (below), so the passing
proof here is ch32fun.

## What runs

All firmware is unmodified ch32fun (`build-ch32fun-examples.sh`,
byte-reproducible), built for `TARGET_MCU=CH32L103`.

| Smoke | Firmware | Checks |
|---|---|---|
| `ch32fun-blink-smoke.yaml` | `examples_l103/blink` | 4/4: PA0, PA2, PA4, PA7 toggle every 250 ms |
| `ch32fun-debugprintf-smoke.yaml` | `examples/debugprintfdemo` | 5/5 on the WCH-Link debug printf channel |

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32l103c8t6/ch32fun-blink-smoke.yaml \
  --output-dir /tmp/l103-blink
```

Blink edges land 24,000,012..14 cycles apart. That is `Delay_Ms(250)` at
ch32fun's default 96 MHz (HSI 8 MHz x12): 3,000,000 HCLK/8 SysTick counts.
Negative control: the same smoke watching PA1, which blink does not drive,
fails 3/4.

## The chip profile

`configs/chips/ch32l103c8t6.yaml`. The L103 keeps the STM32F1 peripheral
bases and the F1 register layouts for RCC, flash interface, AFIO, GPIO,
USART, I2C, SPI, ADC and TIM1..4, and the CH32V20x PFIC IRQ numbers up to 59
(ch32l103.h). So the profile reuses the existing F1 models, the QingKe V4
PFIC and 64-bit SysTick, and the WCH-Link debug printf block. Two new
declarative blocks, both cited in their files:

- `ch32l10x/sys_info.yaml` at 0x1FFFF704: the CHIPID word (0x10310700, the
  C8T6's 0x103107x0 from `ch32l103_dbgmcu.c`, revision nibble 0) and the TS,
  OPA and ADC/HSI_LP trim words (0). WCH's SPL `SystemInit` reads all of them
  in `GPIO_IPD_Unused`.
- `ch32l10x/exten.yaml` at 0x40023800: EXTEN_CTR, storage only. The L10x
  field layout differs from the V20x one (ULLDO_TRIM 10:8, LDO_TRIM 13:12).

## WCH's Arduino core does not run on the CH32L103 as shipped

Measured with the Arduino matrix's stock sketches, built by PlatformIO
`genericCH32L103C8T6` from the same pinned platform and core revisions as the
ch32v203/ch32v307/ch32x035 rows (platform-ch32v f26b06a3, arduino_core_ch32
38206043). Upstream HEAD of both openwch/arduino_core_ch32 and the
Community-PIO-CH32V fork has the same startup file (last changed 2024-06-03,
#107). Evidence: [evidence/arduino-core-upstream/](evidence/arduino-core-upstream/).

**Stock core: 0 of 9 cells pass** ([stock/run_matrix.log](evidence/arduino-core-upstream/stock/run_matrix.log)).
Three defects in the core, each found on the model:

1. **C++ global constructors never run.** `startup_ch32l103.S` has no
   `call __libc_init_array`. The startup of every other family in the core
   has it (V00x, VM00X, V10x, V20x, V30x, X035). So `Serial1`'s constructor
   never stores its vtable pointer. The first `Serial.print` is a virtual
   call through a zero vtable (`c.jalr a5` at 0xc42 with a5 = 0 in the L0
   build). It jumps to 0, which is `j handle_reset`. The chip re-runs reset
   about once per millisecond and never prints. Silicon executes the same
   instruction stream, so this is not a model gap. L0, L1, L2 and L4..L8 all
   end `boot_fail`.
2. **Wire does not compile for L10x.** `libraries/Wire/src/utility/twi.c`
   uses `RCC_APB1Periph_I2C1/2` and `RCC_APB1PeriphClockCmd`. The L10x SPL
   names them `RCC_PB1Periph_*` / `RCC_PB1PeriphClockCmd`. The SPI library
   already has a `CH32L10x` branch for this; Wire does not. L3 is
   `compile_fail`.
3. **The default SPI pins have no peripheral.** `variant_CH32L103C8T6.h`
   sets `PIN_SPI_MOSI/MISO/SCK` to PA7/PA6/PA5 (SPI1). `PeripheralPins.c`
   maps only SPI2 on PB15/PB14/PB13. `SPI.begin()` on the defaults binds
   nothing, and `SPI.transfer` returns 0 (L4 reads frame 0x00000000).

**With defect 1 fixed** (the X035 startup's three lines, added before
`jal SystemInit`;
[startup_ch32l103.S.diff](evidence/arduino-core-upstream/startup-fixed/startup_ch32l103.S.diff)),
the same sketches give **5 pass** on this profile
([startup-fixed/run_matrix.log](evidence/arduino-core-upstream/startup-fixed/run_matrix.log)):

| cell | result |
|---|---|
| L0_serial_boot | pass |
| L1_serial_loop | pass |
| L2_blink_serial | pass (3 edges on PB12, `-DLED_BUILTIN=PB12`) |
| L3_i2c_sensor | compile_fail (defect 2) |
| L4_spi_sensor | oracle_fail, frame 0 (defect 3) |
| L5_adc | pass |
| L6_pwm | pass: TIM2_CH1 on PA0, 3,000 high cycles per 96,000 = 1 kHz at 3.125 % (128/4096) |
| L7_timer | oracle_fail: the same upstream `getCurrentMicros()` down-counter bug as the other WCH rows |
| L8_can | oracle_fail: the matrix sketch has no CH32L10x CAN branch, and this profile does not model CAN |

This is evidence that the model runs the core once the core is fixed. It is
not a matrix row: no `ch32l103` row is in `validation/arduino-matrix/boards.yaml`,
because the stock core it would build fails every cell. The three fixes are
upstream changes to openwch/arduino_core_ch32. None has been filed.

## Not modeled

- USART2..4, I2C1/I2C2, SPI1/SPI2, ADC1 and TIM1..4 are not on this
  profile's ch32fun smokes. The Arduino evidence above exercises USART1, ADC1
  and TIM2 only through the locally fixed core.
- DMA, CAN, LPTIM, OPA/CMP, TouchKey, USBFS, USB PD, RTC, IWDG/WWDG.
- The low-power HSI_LP clock and Sleep/Stop/Standby. ch32fun's
  `examples_l103/standby` is not run.
- Real factory trim values and the silicon revision nibble. They read 0.
- The clock tree is not derived from RCC: `cpu_hz` states the 96 MHz that
  both ch32fun and WCH's SystemInit configure.
- PFIC nesting and priority grouping.
- Physical-board parity: no CH32L103 was measured.
