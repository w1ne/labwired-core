# WCH CH32V003F4P6 on the CH32V003EVT

**Demand rank:** Tier C strategic vendor bet, not a measured MCU rank in the
JetBrains survey. The measured ranks remain ESP32 (38%), ATmega328 (30%),
STM32F103 (17%), other STM32 (13%), and RP2040 (12%). The latest round makes
WCH's own SDK example run unmodified: it adds the HSI-PLL clock handshake, the
QingKe V2 SysTick that every `Delay_Ms` needs, and the QingKe INTSYSCR CSR that
WCH's startup code writes.

## What these profiles run

The GPIO profile boots RV32EC firmware from the CH32V003F4P6 memory map, stays
at the reset 24 MHz clock, enables GPIOD through APB2PCENR, configures PD0 as
push-pull output, and toggles it. Its smoke asserts at least eight GPIO edges.

The USART profile adds a clock-gated USART1 register block and the CH32V003
default TX route on PD5. Firmware configures PD5 for alternate-function output,
sets 115200 baud from the reset clock, polls TXE, and sends
`CH32V003_USART_OK`; the same run continues toggling PD0. Its smoke asserts
both the serial marker and GPIO edges. USART receive, PLL setup, WCH SDI, other
CH32 peripherals, supply/reset sequencing, and physical-board parity are not
modeled. Stack analysis is unsupported for this architecture.

The I2C profile adds the CH32V003's reset-mapped I2C1 pins (PC1/SDA and PC2/SCL)
and a clock-gated I2C1 register window. Firmware sends the SHT31 status command
`0xF32D`, reads the modeled power-on status `0x8010`, and prints
`CH32V003_SHT31_STATUS_OK` on USART1 only when both bytes match. The smoke
therefore exercises the sensor transaction, not merely the MCU's I2C registers.
I2C slave mode, alternate pin remaps, timing accuracy, CRC checking, sensor
conversion behavior, and physical-board parity remain outside this profile.

The engine reuses its STM32F1-shaped GPIO, USART, and I2C register behavior for
the exercised subset. WCH's I2C1 CR1/CR2/DR event map shares the offsets used by
that controller model; the CH32-specific clock gate and default PC1/PC2 I2C pad
mapping are declared separately. The USART descriptor selects the separate
CH32 PD5 TX pad map. This is not a claim that the CH32V003 clock tree or all of
its peripherals are STM32F1-compatible.

For the GPIO smoke, connect an LED and current-limiting resistor between PD0
and GND. The [WCH-CH32V003EVT board reference](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/resources/CH32V00x-Evaluation-Board-Reference-EN.pdf)
documents the board's LED-row pins; `external_led_pd0` is the explicit test
connection, not a factory-installed PD0 LED. For USART, connect PD5 to a
USB-UART adapter's RX input and share ground. The example uses 115200 8N1.

### Clock bring-up, SysTick delays, and the unmodified WCH SDK example

Earlier profiles timed their LED with a busy loop, so the GPIO example below
(which calls `Delay_Ms`) was not actually exercised: the QingKe SysTick did not
exist in the engine, and firmware that touched it faulted at `0xE000F004`. The
`ch32v003f4p6-systick-smoke` profile adds:

- **QingKe V2 SysTick (STK) at 0xE000F000**: CTLR/SR/CNT/CMP per WCH's
  `core_riscv.h`. It counts up at HCLK/8 or HCLK, latches CNTIF on CMP, and
  honors STRE auto-reload. Both community delay idioms work: the WCH SDK's
  `Delay_Us`/`Delay_Ms` (CMP + poll CNTIF) and ch32v003fun's `DelaySysTick`
  (free-running CNT).
- **RCC clock handshake**: HSION→HSIRDY, PLLON→PLLRDY, and SW→SWS once the
  selected source is ready. That is what `SystemInit` and
  `SetSysClockTo_48MHZ_HSI` poll.
- **QingKe V2 core profile** (`core: qingke-v2a`): RV32EC plus the vendor
  INTSYSCR CSR (0x804) that `startup_ch32v00x.S` writes before `SystemInit`.
- FLASH_ACTLR wait states, and AFIO PCFR1 storage (the SDK's HSE attempt sets
  it).

Two smokes use it:

1. `systick-smoke.yaml` runs a register-level replica of WCH's
   `SetSysClockTo_48MHZ_HSI`, then times one `Delay_Us(100)` and one
   ch32v003fun `DelaySysTick(4800)`. Each prints its OK marker only when its
   poll loop waited a bounded, non-zero time. PD0 then toggles every
   `Delay_Us(100)`, and the committed run shows edges every 4,821 cycles
   (4,800 cycles of delay plus loop overhead).
2. `sdk-gpio-toggle-smoke.yaml` runs **WCH's unmodified GPIO_Toggle example**
   (openwch/ch32v003 at `4e1b743`, built by `build-sdk-gpio-toggle.sh`). It
   prints `SystemClk:24000000`, `ChipID:00000000`, and `GPIO Toggle TEST`.
   PD0 then toggles every 6,000,090 cycles, which is `Delay_Ms(250)` at
   24 MHz plus 90 cycles of loop.

Three results to read correctly:

- **24 MHz, not 48 MHz.** The example's default `SystemInit` tries the 24 MHz
  HSE first. The EVT schematic marks the crystal links R4/R5 `NC(0R)`, so on
  the default fit the HSE never starts and the SDK's timeout falls back to the
  HSI. The model never reports HSERDY, so it takes the same path. A board with
  R4/R5 fitted would run at 48 MHz, and this profile does not model that.
- **ChipID 00000000** is deliberate. The factory chip-ID word is not modeled,
  and the system-area window reads zero rather than a plausible fake.
- **Not modeled in this profile:** the PFIC, so no interrupt is delivered here
  (STIE, SWIE and INTSYSCR are stored only; the `pfic-smoke` profile below
  adds interrupt delivery); the HSE; clock dividers
  as an execution-speed change; and remap side effects of AFIO PCFR1. The
  descriptor's `cpu_hz` is 48 MHz, the replica's PLL target. The SDK example's
  24 MHz shows up only in its own `SystemCoreClock` and delay arithmetic.

The SDK carries no open-source license; its headers restrict use to WCH
microcontrollers. Its sources and the built ELF are therefore not committed:
the build script fetches the pinned commit into a cache and writes the image
under `target/`. The one local addition, `picolibc_stdout.c`, routes picolibc's
`printf` to the SDK's own `_write()`.

### Interrupts: the QingKe V2 PFIC

The `ch32v003f4p6-pfic-smoke` profile adds the PFIC at `0xE000E000`, so the
core takes interrupts the way WCH's startup code sets them up: `mtvec =
_start | 3` (vectored, the table holds handler addresses), INTSYSCR = 3, and
`mstatus.MIE` set before `main`. Modeled:

- **PFIC registers**: ISR/IPR status, IENR/IRER enable, IPSR/IPRR pend,
  IACTR active, IPRIOR (bits 7:6), ITHRESDR threshold, GISR, and the two VTF
  channels that skip the table. Bit `n` is interrupt number `n`, so
  `NVIC_EnableIRQ(SysTick_IRQn)` sets IENR0 bit 12.
- **Trap entry and `mret`**: interrupts vector through their table slot;
  ecall-M uses slot 5, a breakpoint slot 9, other exceptions HardFault (3).
  With INTSYSCR.HWSTKEN the hardware prologue/epilogue (HPE) saves ra, t0-t2
  and a0-a5 on entry and restores them on `mret`. Binaries built with WCH's
  MounRiver GCC, whose `"WCH-Interrupt-fast"` handlers save nothing, rely on
  that.
- **SysTick and software interrupts**: STK `STIE && CNTIF` requests
  interrupt 12 and `SWIE` requests 14. Both are level requests that stay
  asserted until the handler clears CNTIF or SWIE.

Two smokes use it:

1. `pfic-smoke.yaml` runs `firmware_pfic.c` (our `main`, linked with WCH's
   SDK startup and drivers). It takes a 1 kHz SysTick interrupt, checks that
   `NVIC_DisableIRQ` stops it and re-enabling resumes it, then raises the
   software interrupt once through `NVIC_SetPendingIRQ` and once through
   `STK_CTLR.SWIE`. Each marker prints only after its handler ran, and PD0 is
   toggled only from the SysTick ISR.
2. `sdk-systick-interrupt-smoke.yaml` runs **WCH's unmodified
   SYSTICK_Interrupt example**. Its handler only clears `STK_SR`, so the
   assertion is that CNTIF reads 0 after two 1 s compare periods. The
   committed negative control runs the same image on the SysTick profile
   (no PFIC), where CNTIF stays at 1.

Upstream GCC does not know WCH's `"WCH-Interrupt-fast"` attribute. It warns and
drops the attribute, so the handler returns with `ret`, which is also wrong on
silicon. `build-sdk-interrupt-examples.sh` compiles with
`-D'interrupt(x)=interrupt'`, which turns those handlers into standard RISC-V
interrupt handlers (software save plus `mret`, as ch32v003fun writes them).
The SDK sources are unchanged.

Not modeled: interrupt nesting (INESTEN is stored, and a pending interrupt
waits for `mret`), the key-gated system reset that the SDK's HardFault handler
requests, sleep-on-exit and WFE events, the NMI, and interrupts from the other
peripherals (USART1, I2C1, EXTI, ...), which declare no `irq:` line in these
profiles. QingKe V2 silicon pushes the HPE frame onto the stack. Here the frame
is held inside the core, so registers come back exactly but the stack memory
is not written.

### ch32fun: the community toolchain, unmodified

[ch32fun](https://github.com/cnlohr/ch32fun) (CNLohr, MIT) is the toolchain
most hobbyist CH32V003 code is written against. The
`ch32v003f4p6-ch32fun-smoke` profile runs twenty of its examples unmodified,
built by `build-ch32fun-examples.sh` with the flags `ch32fun.mk` uses for
`TARGET_MCU=CH32V003` (pinned commit `b28d285`). ch32fun is MIT licensed, so
the ELFs are committed under [`ch32fun/`](ch32fun/) with its license.

| Example | What the smoke checks | Committed run |
| --- | --- | --- |
| `blink` | PD0, PD4, PD6 and PC0 toggle together; 7 edges each in 90 M cycles | edges every 12,000,006 to 12,000,009 cycles, i.e. `Delay_Ms(250)` at 48 MHz |
| `uartdemo` | `Count: 0` .. `Count: 3` in order on USART1 (PD5), PD0 toggling | PD0 edges every 2,400,004 cycles (50 ms) |
| `systick_irq` | debug-printf banner, `Milliseconds taken: 500` twice, DMDATA0 never timed out, PD0/PC0 edges | the 1 kHz SysTick ISR counts exactly 500 ms across two `Delay_Ms(250)` |
| `adc_polled` | a potentiometer on PD4 (AIN7) is turned 50 % → 25 % → 100 %; the printed codes must be `511`, `511`, `255`, `255`, `1023` | `Count: 0 adc: 511` .. `Count: 4 adc: 1023`, one sample every 200 ms |
| `i2c_oled` | a 128x32 SSD1306 at 0x3C on I2C1 (PC1/PC2); after the half-lit "buffer fill with binary" frame, mode 1 must leave exactly the two diagonals `(x, x/4)` and `(x, 31 - x/4)` lit: 256 of 4096 pixels, corners, centre crossing and two empty bands checked | 12/12; every packet completes with no `ssd1306_i2c_error` timeout |
| `spi_oled` | a 0.96" 128x64 SSD1306 module strapped for 4-wire SPI on SPI1 (PC5 SCK, PC6 MOSI), CS# on PC3, D/C# on PC4, RES# on PC2 (driven, not modelled), wired as the example's README lists. The example fills the frame from its LFSR `rand()` and runs Conway's Life, one full refresh per generation. The smoke checks the console and three exact quadrant pixel counts of generation 21. [`check-spi-oled.py`](check-spi-oled.py) recomputes every frame from the example's source and matches each latched frame's SHA-256 ([`check-spi-oled.log`](evidence/ch32fun-spi-oled-smoke/check-spi-oled.log)) | 7/7; 23 frames in order, all exact: the blank init refresh, the random fill, then Life generations 1..21 about 48 ms apart. The stop lands four GDDRAM bytes into generation 22's refresh |
| `tim2_pwm` | TIM2 PWM on CH1 (PD4), CH2 (PD3) and CH4 (PD7), with an LED on each; debug-printf banner and the TIM2 setup left behind (ATRLR, CEN/ARPE, CHCTLR1/2, CCER). [`check-tim2-pwm.py`](check-tim2-pwm.py) decodes the waveform of a `--watch-gpio` run, committed as [`pwm-capture.json.gz`](evidence/ch32fun-tim2-pwm-smoke/pwm-capture.json.gz) with its output in [`pwm-check.log`](evidence/ch32fun-tim2-pwm-smoke/pwm-check.log) | 11,546 whole 256-cycle periods (187.5 kHz); CH1 low for `count`/256 and CH2 low, CH4 high for `(count + 128) & 255`/256, through CH2's 255 → 0 wrap; `count` steps by 1 every 937.5 periods, i.e. `Delay_Ms(5)` |
| `tim1_pwm` | TIM1, the advanced-control timer: PWM on the complementary output CH1N (PD0) and on CH4 (PC4), MOE set, an LED on each; debug-printf banner and the TIM1 setup left behind (ATRLR, CEN, CHCTLR1/2, CCER, BDTR.MOE). [`check-tim1-pwm.py`](check-tim1-pwm.py) decodes a `--watch-gpio` run, committed as [`pwm-capture.json.gz`](evidence/ch32fun-tim1-pwm-smoke/pwm-capture.json.gz) with its output in [`pwm-check.log`](evidence/ch32fun-tim1-pwm-smoke/pwm-check.log) | 16,851 whole 256-cycle periods (187.5 kHz); CH1N low for `count`/256 and CH4 low for `(count + 128) & 255`/256 through CH4's wrap; `count` steps by 1 every 937.5 periods, i.e. `Delay_Ms(5)` |
| `tim1_pwm_complementary_outputs` | TIM1 remapped by `AFIO_PCFR1.TIM1_RM` = 01: the complementary pair CH1 (PC6) / CH1N (PC3), both active low, with an LED on each; the main loop steps `BDTR.DTG` every 20 ms. The smoke checks the remap, the pad modes and the TIM1 setup left behind (PSC, ATRLR, CH1CVR, CHCTLR1, CCER, CEN, MOE); the example prints nothing. [`check-tim1-complementary.py`](check-tim1-complementary.py) decodes a 265 M-cycle `--watch-gpio` run, committed as [`pwm-capture.json.gz`](evidence/ch32fun-tim1-complementary-smoke/pwm-capture.json.gz) with its output in [`pwm-check.log`](evidence/ch32fun-tim1-complementary-smoke/pwm-check.log) | 2,600 whole 100,010-cycle periods (479.95 Hz), OC1REF high 50,000 cycles; the two pads are never low together, and both dead-time gaps decode to `DTG` stepping +1 through all 256 values (DT 0..1008 cycles over all four encodings) and the 255 → 0 wrap, each held a mean 959,947 cycles (`Delay_Ms(20)`) |
| `exti_pin_change_isr` | a button on PD3 is pressed, released and pressed again; the EXTI7_0 handler blips PC1. The smoke checks PC1 edges and the EXTI/AFIO/PFIC state left behind; the `--watch-gpio` run is committed as [`pc1-capture.json`](evidence/ch32fun-exti-smoke/pc1-capture.json) | PD3 rises at 200,001 and 600,001 and PC1 blips at 200,007-200,009 and 600,007-600,009: exactly 4 PC1 edges, none at the release (falling edge, FTENR clear) at 400,001 |
| `spi_max7219` | 16-bit SPI1 frames (PC5 SCK, PC6 MOSI) to a MAX7219 with LOAD on PD0; the panel state the init writes leave ([`max7219-state.json`](evidence/ch32fun-spi-max7219-smoke/max7219-state.json)) | 10/10: intensity 15, scan limit 7, shutdown clear, 12 of 64 LEDs lit as Code-B 7..0 |
| `adc_dma_opamp` | ADC1 scans AIN7 (PD4), AIN4 (PD3), AIN3 (PD2) and AIN2 (PC4) in continuous mode, and DMA1 channel 1 (circular, half-words) moves every conversion into the example's four-entry buffer, which `main()` prints every 200 ms. Pots on PD3/PD2/PC4 start at 100/50/0 % and move to 25/75/100 % at 30 M cycles. The smoke checks both printed rows twice each, the banner lines and `EXTEN_CTR.OPA_EN`. PD4 is the op-amp output; the OPA is not modelled and nothing is wired there, so that column is not checked. Uses [`ch32v003evt-ch32fun-adc-dma-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-adc-dma-smoke.yaml) | 8/8 over 55 M cycles: ` 1023  511    0`, then `  255  767 1023` after the move, each column on its own channel |
| `uart_tx_dma` | `Hello World!\r\n` three times on USART1 (PD5), every byte moved by DMA1 channel 4; the transfer-complete handler (PFIC 25) toggles PD6 | 5/5: 42 bytes, PD6 4 edges, CNTR4 = 0, INTFR nibble 4 = 0x5 |
| `usart_dma_rx_circular` | four command lines (`toggle`, `bogus`, `toggle`, `toggle`, 31 bytes) injected on USART1 RX (PD6); CTLR3.DMAR moves every byte through DMA1 channel 5 into the example's 16-byte circular buffer, so the last two lines wrap past its end. The smoke checks the replies on USART1 TX, three PC7 toggles, CNTR5, CFGR5 and the buffer image left behind | 7/7: `Horay!`, `Invalid cmd: bogus`, `Horay!`, `Horay!`; PC7 ends high, CNTR5 = 1 (one lap + 15 items), rx_buf = `oggle\r\ntoggle\r\nt` |
| `ws2812bdemo` | a 191-LED WS2812B strip on PC6, fed by SPI1 at 3 MHz from DMA1 channel 3 in circular mode with HT/TC refills (PFIC 24). The strip decodes PC6's pulse widths; [`check-ws2812.py`](check-ws2812.py) recomputes every LED from the demo's own colour tables and replays the driver's buffer protocol ([`check-ws2812.log`](evidence/ch32fun-ws2812-smoke/colour-check/check-ws2812.log)) | 3/3 and all 190 LEDs the driver sends match frame 9. The driver clocks out 190 of `NR_LEDS` 191: the HT refill that writes LED 190 is the one that clears CIRC, so that half is never sent. [CH32V003F4P6 status and sourcing](evidence/ch32fun-ws2812-smoke/component-status.md) |
| `iwdg` | the independent watchdog: PD0/PD4/PC0 on 1.5 s, off 1 s, the IWDG started at /128 with RLDR 0xFFF, on 3 s, one feed, then 250 ms blinks with no further feed until the watchdog resets the chip. The smoke checks the edge counts and that PSCR is back at 0 after the reset (11.25 s, 540 M cycles). [`check-iwdg.py`](check-iwdg.py) decodes PD0 from a `--watch-gpio` run, committed as [`iwdg-capture.json.gz`](evidence/ch32fun-iwdg-smoke/iwdg-capture.json.gz) with its output in [`iwdg-check.log`](evidence/ch32fun-iwdg-smoke/iwdg-check.log) | feed at 5,500.003 ms, 17 blink edges at 250 ms, then the reset at feed + 4,095.002 ms (4095 counts of LSI/128 at 128 kHz) releases PD0, and the second boot's 1,500 ms on pulse starts 94 cycles later, as on power-up |
| `tim2_encoder` | TIM2 in encoder mode 3 (`SMCFGR.SMS` = 011, both inputs, x4) on partial remap 1: a switch encoder's A on PC5 (T2CH1) and B on PC2 (T2CH2), both pulled up by the example and closing to GND. Three detents forward from cycle 6,000,000 and five back from 13,000,000; `main()` prints the position when `CNT` moved. The smoke checks both printed lines, the remap, SMS, CEN and `CTLR1.DIR`, and the final `CNT`; the whole terminal is in [`terminal.log`](evidence/ch32fun-tim2-encoder-smoke/terminal.log) | 7/7; `relative=12 absolute=36875 delta=12`, then `relative=-8 absolute=36855 delta=-20`, nothing else: `CNT` sits at 0x8FFF until the first edge |
| `tim2_pwm_remap` | TIM2 PWM (ATRLR 255, CH1/CH2 active low, CH2 = 255 - CH1) with LEDs on PD4, PD3, PC1 and PC7; after 255 duty steps `ToggleRemap()` sets `TIM2_RM` = 11 while TIM2 runs. The smoke checks the console (`Full Remapping!`), the remap and the TIM2 setup; [`check-tim2-pwm-remap.py`](check-tim2-pwm-remap.py) decodes a `--watch-gpio` run, committed as [`pwm-capture.json.gz`](evidence/ch32fun-tim2-pwm-remap-smoke/pwm-capture.json.gz) with its output in [`remap-check.log`](evidence/ch32fun-tim2-pwm-remap-smoke/remap-check.log) | 9/9; PD4/PD3 carry counts 253..254 and stop at cycle 66,011,469, PC1/PC7 start 255 cycles later at count 0 and carry 0..16; every whole period is 256 cycles with CH1 low + CH2 low = 255 |
| `tim1_tim2_cascade` | timer master/slave: TIM1 counts milliseconds (PSC 47999, ATRLR 999) and sends TRGO on every update (`CTLR2.MMS` = 010); TIM2 counts that TRGO on ITR0 in external clock mode 1 (`SMCFGR` TS = 000, SMS = 111), so it counts seconds. `main()` prints `TIM2.TIM1 s` every 500 ms on the debug-printf channel. The smoke checks all eight lines, the TIM1/TIM2 setup and TIM2 `CNT` = 3 at the stop | 15/15 over 170 M cycles (3.54 s): `0.000 s`, `0.500 s` .. `3.500 s`. Before the TRGO/ITR link, SMS = 111 still counted CK_INT, so the seconds field was HCLK cycles mod 65536 |
| `input_capture` | TIM1 input capture at 1 us per count: a pulse source on PD2 (T1CH1) gives a 1000 us and a 2500 us pulse; CH1 latches the rising edge, CH2 (TI1, `CC2P`) the falling edge, and the TIM1_CC handler (PFIC 37) queues each capture for `main()` to print. The smoke checks the four printed captures and the TIM1 setup left behind (PSC, CHCTLR1, CCER, DMAINTENR, CEN); the whole terminal is in [`terminal.log`](evidence/ch32fun-input-capture-smoke/terminal.log) | 11/11; fall minus rise is exactly 1000 (24993 → 25993) and 2500 (21957 → 24457) counts, one handler entry per capture, no `badtrigger` |

This profile adds these to the PFIC profile:

- **GPIOA and GPIOC**, because `funGpioInitAll()` clocks all three ports and
  the examples drive PC0.
- **The WCH-LinkE debug-printf channel** (`wch_debug_printf` at
  `0xE00000F4`). ch32fun's default `printf` does not use a UART: it hands up
  to 7 characters at a time to the probe through the debug-module registers
  DMDATA0/DMDATA1 over the single SWIO wire, and `minichlink -T` prints them.
  The model plays the probe's part: it takes each request when the firmware
  raises DMDATA0 bit 7, acknowledges by writing 0, and reads 0 on the
  "debugger attached" sentinel at `0xE00000FC`. The printed text is the
  `terminal` log of the `debug_printf` peripheral, so a smoke checks it with
  `peripheral_log: { peripheral: debug_printf, log: terminal, contains: ... }`.
- **ADC1** (`0x40012400`): the STM32F1 ADC block, which the CH32V003 copies
  register for register (RSTCAL/CAL handshakes, SWSTART with EXTSEL = 111,
  RSQR3, RDATAR), at the CH32V003's 10 bits. The `adc_polled` smoke uses the
  system [`ch32v003evt-ch32fun-adc-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-adc-smoke.yaml),
  which wires a `potentiometer` to AIN7, and turns it with test-script
  `stimuli`. Its negative control
  ([`evidence/ch32fun-adc-polled-smoke/negative-control/`](evidence/ch32fun-adc-polled-smoke/negative-control/))
  puts the same image on the 12-bit STM32F1 ADC: it boots, calibrates and
  prints `2047`, `1023`, `4095`, so every code check fails.
  With `CTLR1.SCAN` the regular group is SQ1..SQ(L) from RSQR3/RSQR2/RSQR1
  (L = `RSQR1[23:20]` + 1), with EOC and EOCIE at the end of the group.
  `CTLR2.DMA` requests DMA1 channel 1 after every conversion, and CONT
  restarts the group without clearing EOC. `adc_dma_opamp` uses all three.
- **EXTEN** (`0x40023800`): `EXTEN_CTR`/`EXTEN_KEYR` as plain storage, a
  declarative block ([`exten.yaml`](../../configs/peripherals/ch32v003/exten.yaml)),
  so `adc_dma_opamp`'s `OPA_EN` store lands and reads back. Nothing else
  happens: the op-amp, the PLL delay, LOCKUP and the trims are not modelled.
- **I2C1** (`0x40005400`, PC1 SDA, PC2 SCL): the engine's STM32F1-shaped
  I2C master, which the CH32V003 matches register for register. The
  `i2c_oled` smoke uses the system
  [`ch32v003evt-ch32fun-i2c-oled-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-i2c-oled-smoke.yaml),
  which hangs a 128x32 SSD1306 module on the bus. Getting it to draw took two
  fixes to that shared model, both pinned by unit tests in `i2c_tests.rs`.
  START now sets SR2.MSL and SR2.BUSY along with SR1.SB (RM0008 §26.3.3).
  Without that, ch32fun's driver timed out waiting for the master-mode event
  and the panel stayed blank. A 16-bit store to the data register now sends
  one byte. ch32fun declares `DATAR` as `uint16_t`, and the old byte-sliced
  store sent every byte twice, which drew the diagonals at half slope.
- **SSD1306 on SPI1** (`spi_oled`): the SPI master is the one `spi_max7219`
  already uses. The new part is the panel,
  [`configs/devices/ssd1306_spi.yaml`](../../configs/devices/ssd1306_spi.yaml)
  (`oled-ssd1306-spi`), the 7-pin SPI module. The engine's D/C-pin display
  path assumed MIPI framing, where a command's arguments ride D/C high. The
  SSD1306 sends them with D/C# low (datasheet section 8.1.3), so the
  declarative display spec gained `dc.params: command`. Without it, the
  driver's `21 00 7F` decodes as three opcodes and the window stays at
  reset. ch32fun also raises CS# after every byte, so the panel declares
  `cs_select: keeps_stream`. A unit test in `declarative_display.rs` breaks
  each rule in turn. The smoke uses
  [`ch32v003evt-ch32fun-spi-oled-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-spi-oled-smoke.yaml).
- **TIM2** (`0x40000000`): the engine's STM32F1 general-purpose timer,
  whose CTLR1/CHCTLRx/CCER/PSC/ATRLR/CHxCVR layout and PWM modes the
  CH32V003 shares, counting at HCLK. Two changes made `tim2_pwm` drive its
  pins. The CH32V003 RCC model kept only I2C1 in `APB1PCENR`, so the TIM2
  clock enable was dropped and the timer never ran; it now keeps TIM2EN
  (bit 0) too. The F1 timer pad router only knew the STM32F103's pads
  (TIM2 on PA0..PA3); it now picks the CH32V003 reset mapping (T2CH1..4 on
  PD4, PD3, PC0, PD7) when the RCC profile is `ch32v003`. The test
  [`crates/core/tests/ch32v003_tim2_pads.rs`](../../crates/core/tests/ch32v003_tim2_pads.rs)
  pins the routes and fails if the F103 table is used. The smoke uses
  [`ch32v003evt-ch32fun-tim2-pwm-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-tim2-pwm-smoke.yaml).
  CH4 comes out active high while CH1/CH2 are active low. That is the
  example, not the model: it sets polarity with `TIM_CCxP & TIM2_DEFAULT`,
  and `TIM_CC4P & 0xff` is 0, so silicon does the same. At each duty step,
  CH2/CH4 can take the new count one period after CH1, because the loop
  stores CH1CVR before the other two and the compare registers are
  preloaded. The waveform check allows exactly that one period.
- **TIM1** (`0x40012C00`): the engine's advanced-control timer
  (`advanced: true`), counting at HCLK. Three changes made `tim1_pwm`
  drive its pins. The CH32V003 RCC model dropped TIM1EN (`APB2PCENR` bit
  11), so every TIM1 register read back 0; it now keeps it. The timer model
  stored `BDTR` but never generated the complementary outputs, and the
  example's only channel besides CH4 is CH1N on PD0. The model now
  generates OC1N..OC3N (RM0008 Table 83): with `MOE` set, `CCxNE` alone puts
  `OCxREF` itself on the pad (`tim1_pwm` sets `CC1NE|CC1NP` with `CC1E`
  clear, so PD0 is `OC1REF` inverted); `CCxE|CCxNE` gives the complementary
  pair, each rising edge delayed by the `DTG` dead time in CK_INT x `CKD`
  cycles; `OSSR`/`OSSI` with `OISx`/`OISxN` give the off and idle levels.
  The F1 timer pad router gains the CH32V003 TIM1 reset mapping: T1CH1..4
  on PD2, PA1, PC3, PC4 and T1CH1N..3N on PD0, PA2, PD1. The unit tests in
  `timer.rs` pin the pair inversion, the lone-`CCxNE` rule, dead time at a
  prescaler that does not divide it, `CKD`, and the idle/off levels;
  [`crates/core/tests/ch32v003_tim1_pads.rs`](../../crates/core/tests/ch32v003_tim1_pads.rs)
  pins all seven routes. The smoke uses
  [`ch32v003evt-ch32fun-tim1-pwm-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-tim1-pwm-smoke.yaml).
- **AFIO `PCFR1` TIM1 remap** (`TIM1_RM`, bits [7:6]). The TIM1 pad
  routes are bound once per remap setting and each is live only while
  `PCFR1` selects it. The AFIO block shares the register with the pads
  (`Peripheral::remap_register`, read lock-free through a `RemapGate` on the route).
  Partial remap 1, which ch32fun's `tim1_pwm_complementary_outputs` uses,
  puts CH1/CH1N on PC6/PC3, CH2/CH2N on PC7/PC4, CH3 on PC0 and CH4 on
  PD3. Partial remap 2 uses the reset pads. Full remap puts CH1/CH2/CH3 on
  PC4/PC7/PC5, CH4 on PD4 and CH1N/CH2N/CH3N on PC3/PD2/PC6. The pads come
  from ch32fun's `ch32v003hw.h` (`AFIO_PCFR1_TIM1_REMAP_*`), checked against
  the example's own table. Those two disagree on full-remap CH4 (PD4 vs
  PD2); the reference manual's `TIM1_RM` field text, quoted verbatim in
  [ch32-rs/ch32-data](https://github.com/ch32-rs/ch32-data)
  `data/family/CH32V003.yaml`, settles it on PD4.
- **AFIO `PCFR1` TIM2 remap** (`TIM2_RM`, bits [9:8]), all four settings:
  00 CH1/ETR PD4, CH2 PD3, CH3 PC0, CH4 PD7; 01 PC5, PC2, PD2, PC1; 10 PC1,
  PD3, PC0, PD7; 11 PC1, PC7, PD6, PD5. The RM field text (same ch32-data
  file) and the tables in ch32fun's `tim2_pwm_remap` and `tim2_encoder`
  agree on all sixteen pads; `ch32v003hw.h` has two typos ("PD4" for CH4 at
  00 and CH3 at 01) and is not used. A `PCFR1` store re-syncs every GPIO
  port's routes (`SystemBus::remap_changed`), so a remap written while the
  timer runs moves the PWM outputs and logic-capture taps immediately. Timer
  input levels are reported from the selected pad when their level differs
  from the last level reported for that route; a route that returns at its
  previously reported level can leave a timer input's remembered level stale
  until that pad next changes. The checked encoder example keeps remap 01
  fixed while it runs.
  [`crates/core/tests/ch32v003_tim2_pads.rs`](../../crates/core/tests/ch32v003_tim2_pads.rs)
  checks the four output mappings and that no extra TIM2 pad is live. The
  unmodified `tim2_pwm_remap` smoke checks the register setup and console, and
  its [waveform capture](evidence/ch32fun-tim2-pwm-remap-smoke/pwm-capture.json.gz)
  verifies the PWM pair moves from PD4/PD3 to PC1/PC7 while the timer runs.
  [`crates/core/tests/ch32v003_tim2_encoder.rs`](../../crates/core/tests/ch32v003_tim2_encoder.rs)
  checks encoder counts from the pulled-up PC5/PC2 inputs; its unmodified
  firmware smoke is [`ch32fun-tim2-encoder-smoke.yaml`](ch32fun-tim2-encoder-smoke.yaml).
  [`crates/core/tests/ch32v003_tim1_pads.rs`](../../crates/core/tests/ch32v003_tim1_pads.rs)
  pins partial and full remap and the move back to reset. The smoke uses
  [`ch32v003evt-ch32fun-tim1-complementary-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-tim1-complementary-smoke.yaml).
- **TIM1/TIM2 input capture and TIM2 encoder mode.** The capture router uses
  the same CH32V003 pad table as the outputs: a channel pad that is a digital
  input feeds that channel's TIy while `PCFR1` selects its mapping. TIM2
  encoder SMS modes 001/010/011 count the selected TI1/TI2 edges, with
  direction from the other input, CCxP polarity and the input prescaler.
  `tim2_encoder` checks x4 switch-encoder counts on remap 01. Before this, the
  router used the STM32F103 table, which puts TIM1 on PA8..PA11 (not on
  this package) and TIM2 CH2/CH3 on PA1/PA2 (TIM1 pads here). TIM1 has
  `irq: 37` (TIM1_CC): on a QingKe chip a level source's own line now
  drives the PFIC request LEVEL, so the CH1CVR read that clears CC1IF also
  withdraws the request. With a latched pend instead, ch32fun's
  `input_capture` entered its handler a second time and printed
  `badtrigger`.
  [`crates/core/tests/ch32v003_tim1_capture.rs`](../../crates/core/tests/ch32v003_tim1_capture.rs)
  pins the PD2 route, the level drop on the read, the remap, and TIM2's
  CH32V003 pads. The smoke uses
  [`ch32v003evt-ch32fun-input-capture-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-input-capture-smoke.yaml).
- **TIM1 → TIM2 cascade (TRGO / ITR0).** The engine had no link between
  timers, and SMS = 111 (external clock mode 1) was not decoded, so TIM2 kept
  counting CK_INT. Now a chip yaml names each slave's internal triggers,
  `config: { itr: [tim1] }` here (ITR0 = TIM1, as `tim1_tim2_cascade`
  selects with `TIM_TS_ITR0`), and a master with `CTLR2.MMS` = 010 sends
  one TRGO pulse per update event, a counter wrap or an EGR.UG. The master
  publishes a closed-form plan of its future updates whenever firmware
  changes it, so the slave reads its pulse count at any cycle without the
  master being advanced, and the legacy walk counts pulses as they happen.
  In external clock mode 1 the slave's prescaler and counter step on TRGI,
  either those pulses on ITRx or the TI edges for TS = 100..110. Not
  modeled: the other MMS sources (reset, enable, compare pulse, OCxREF),
  the gated and trigger slave modes on ITRx, ETR, and scheduled interrupts
  for an externally clocked slave: its flags latch when it is read, so
  UIE on TIM2 in this mode would not wake the core. Unit tests in
  `timer.rs` pin the pulse count, a mid-run master reconfiguration, the
  slave prescaler and wrap, MMS gating, the no-CK_INT rule, and the walk.
- **IWDG** (`0x40003000`): the STM32 independent watchdog block
  (CTLR/PSCR/RLDR/STATR at the KR/PR/RLR/SR offsets) with
  `config: { lsi_hz: 128000 }`, which turns on its counting mode. Key
  `0xCCCC` starts a 12-bit down-counter from `0xFFF` at LSI / (4 << PSCR),
  `0xAAAA` reloads it from RLDR, and `0x5555` unlocks PSCR/RLDR. When the
  counter reaches 0, the model asks the machine for a CPU reset (cause
  `Watchdog`), which reboots the core through the same path as a Cortex-M
  `SYSRESETREQ`. The reset stops the watchdog again and puts every
  CH32V003 GPIO port back to floating input with OUTDR = 0, so an LED the
  firmware lit goes dark across the reboot instead of staying on. Not
  modeled: the RCC_RSTSCKR reset flags (IWDGRSTF is not recorded), the
  PVU/RVU busy flags, the hardware-watchdog option byte, debug-halt freeze,
  and the WWDG. Without `lsi_hz` (every STM32 descriptor today) the IWDG
  stays a register latch that never resets. The smoke uses
  [`ch32v003evt-ch32fun-iwdg-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-iwdg-smoke.yaml).
- **EXTI** (`0x40010400`) and the CH32V003 **`AFIO_EXTICR`**. EXTI is the F1
  register file (INTENR/EVENR/RTENR/FTENR/SWIEVR/INTFR) over ten lines, but
  the port select is not: the CH32V003 has one `AFIO_EXTICR` at `0x08` with
  a 2-bit field per line 0..7 (`AFIO_EXTICR_EXTI3_PD` = `0xC0`), where the
  F103 has four EXTICRs of 4-bit fields. The `afio` model takes
  `profile: ch32v003` for that layout, and the `exti` model takes
  `profile: ch32v003`, which drives PFIC interrupt 20 (`EXTI7_0_IRQn`) as a
  level from `INTFR & INTENR` over lines 0..7, like the QingKe SysTick. The
  handler's `INTFR` write-1-clear drops the request on the same store, so it
  runs once per edge. The test
  [`crates/core/tests/ch32v003_exti.rs`](../../crates/core/tests/ch32v003_exti.rs)
  pins the port select, the edge polarity, INTENR gating and SWIEVR. Its
  negative control
  ([`evidence/ch32fun-exti-smoke/negative-control/`](evidence/ch32fun-exti-smoke/negative-control/))
  decodes `AFIO_EXTICR` the F103 way: `0xC0` then selects port A for line 3,
  the handler never runs, PC1 never moves, and the smoke and two of the three
  tests fail. The smoke uses
  [`ch32v003evt-ch32fun-exti-smoke.yaml`](../../configs/systems/ch32v003evt-ch32fun-exti-smoke.yaml),
  which wires a `button` to PD3 and presses it with test-script `stimuli`.

The debug-printf negative control
([`evidence/ch32fun-systick-irq-smoke/negative-control/`](evidence/ch32fun-systick-irq-smoke/negative-control/))
runs the same `systick_irq` image with that window as plain storage, which is
a board with no probe attached. The GPIO checks still pass, but the firmware's
`printf` waits for an acknowledgement that never comes, times out and ORs
`0xC0` into DMDATA0, so the DMDATA0 check fails.

Read these results correctly:

- The modeled probe acknowledges on the same store, so the firmware never
  waits for it. A real WCH-LinkE polls over USB, which makes `printf` slower
  on silicon; the text is the same.
- USART1 transmit is not paced at the baud rate. Each `Count: N` line costs
  about 600 cycles here, where 9 characters at 115200 baud take about 37,500
  cycles on silicon. Line contents and order are exact; their timing is not.
- Not modeled: input from the host over the debug channel (ch32fun's
  `handle_debug_input`), a detached probe as a configuration, and every
  peripheral the other ch32fun examples need (OPA, watchdogs,
  standby; SPI1 slave mode and its interrupt; DMA requests other than
  SPI1 TX and USART1 TX).
- On WS2812: the strip latches a frame when the next frame's first rising
  edge proves the 40 us reset gap, because it sees only edges. A real LED
  latches on the gap itself, so between frames the model shows the previous
  frame. The pulse widths come from the SPI bit engine's cycle stamps; DMA
  items cost no CPU cycles, so ISR refill latency is not stressed the way it
  is on silicon. Negative controls in
  [`evidence/ch32fun-ws2812-smoke/negative-control/`](evidence/ch32fun-ws2812-smoke/negative-control/):
  no SPI pad map gives 0 LEDs; no CNTR re-arm gives 6 LEDs and CIRC stuck.
- On EXTI: the 6-cycle gap from the PD3 edge to the PC1 store is the
  model's interrupt entry plus two instructions, not a measured QingKe V2
  latency, and the input filter/synchroniser delay is not modeled (the flag
  sets on the edge's cycle). Event mode (EVENR) wakes nothing, because sleep
  is not modeled; lines 8 (PVD) and 9 (AWU) latch and clear but have no
  source and raise no interrupt.
- On I2C1: bus timing (`CKCFGR` is stored, and transfers complete
  immediately), the `RCC_APB1PRSTR` peripheral reset, the AFIO remaps,
  slave mode, PEC/SMBus, the I2C interrupts and DMA are not modeled. The
  `i2c_oled` run stops in mode 1, about 2.2 s in; modes 2 to 8 (lines,
  circles, image, text) take another 14 s of simulated time and are not
  checked.
- On ADC1: sample time (`SAMPTR`) does not change the conversion time
  (every conversion takes 14 HCLK cycles, faster than silicon), and the
  injected group, discontinuous mode, external triggers, the analog
  watchdog, `CTLR2.ALIGN`, the internal V_REFINT/V_CAL channels and
  calibration offsets are not modeled.
- The OPA is not modeled: `EXTEN_CTR.OPA_EN` is stored, but no op-amp output
  reaches PD4. Codes are ideal (no
  offset, gain or noise error) against a 3.3 V reference.
- On TIM1: not modeled: the break input (BKIN) and
  `BDTR.AOE`/`LOCK`, center-aligned and down counting, the repetition
  counter as an update divider, DMA, the TIM1 break/update/trigger
  interrupts and the `RCC_APB2PRSTR` reset. `tim1_pwm` does not set
  `OCxPE`, so on silicon a compare write takes effect mid-period; the model
  applies every duty change at the next period boundary, which the waveform
  check sees as at most one period of lag on CH4. The capture window is
  `count` 116 to 134. `tim1_pwm` never enables both halves of a pair;
  dead time is checked by `tim1_pwm_complementary_outputs` instead.
- On TIM2: the `RCC_APB1PRSTR` reset, PD7's NRST option byte (the pad is
  treated as GPIO, as after `minichlink -d`), the ETR input, the
  external-clock, gated and trigger slave modes, DMA and TIM2 interrupts are
  not modeled (TIM2 input captures and encoder counts land, but only polling
  sees them). Encoder mode has no input filter delay beyond `ICxF`'s, and
  a remap between two pads at different levels reports the newly selected
  pad only once its level differs from what that route last reported. The
  logic capture keeps the newest ~21,800 edges per pad, so the decoded
  window is `count` 121 to 134 of the 0 to 255 ramp (about 60 ms, including
  CH2's wrap through 0). Earlier steps only reach the edge-count checks.

## Copy-paste ch32fun blink

ch32fun's own [`examples/blink`](https://github.com/cnlohr/ch32fun/blob/master/examples/blink/blink.c),
unchanged (MIT). Put it in `examples/blink/` of a ch32fun checkout and run
`make`; the simulator runs the same ELF.

```c
#include "ch32fun.h"
#include <stdio.h>

#define PIN_1 PD0
#define PIN_K PD4
#define PIN_BOB PD6
#define PIN_KEVIN PC0

int main()
{
	SystemInit();

	funGpioInitAll();

	funPinMode( PIN_1,     GPIO_Speed_10MHz | GPIO_CNF_OUT_PP );
	funPinMode( PIN_K,     GPIO_Speed_10MHz | GPIO_CNF_OUT_PP );
	funPinMode( PIN_BOB,   GPIO_Speed_10MHz | GPIO_CNF_OUT_PP );
	funPinMode( PIN_KEVIN, GPIO_Speed_10MHz | GPIO_CNF_OUT_PP );

	while(1)
	{
		funDigitalWrite( PIN_1,     FUN_HIGH );
		funDigitalWrite( PIN_K,     FUN_HIGH );
		funDigitalWrite( PIN_BOB,   FUN_HIGH );
		funDigitalWrite( PIN_KEVIN, FUN_HIGH );
		Delay_Ms( 250 );
		funDigitalWrite( PIN_1,     FUN_LOW );
		funDigitalWrite( PIN_K,     FUN_LOW );
		funDigitalWrite( PIN_BOB,   FUN_LOW );
		funDigitalWrite( PIN_KEVIN, FUN_LOW );
		Delay_Ms( 250 );
	}
}
```

## Copy-paste GPIO example

This follows WCH's official [GPIO_Toggle sample](https://github.com/openwch/ch32v003/blob/main/EVT/EXAM/GPIO/GPIO_Toggle/User/main.c).
The full WCH SDK supplies `debug.h` and delay helpers.

```c
#include "debug.h"

int main(void) {
    u8 state = 0;
    GPIO_InitTypeDef gpio = {0};

    SystemCoreClockUpdate();
    Delay_Init();
    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOD, ENABLE);
    gpio.GPIO_Pin = GPIO_Pin_0;
    gpio.GPIO_Mode = GPIO_Mode_Out_PP;
    gpio.GPIO_Speed = GPIO_Speed_30MHz;
    GPIO_Init(GPIOD, &gpio);

    while (1) {
        Delay_Ms(250);
        GPIO_WriteBit(GPIOD, GPIO_Pin_0,
                      (state == 0) ? (state = Bit_SET) : (state = Bit_RESET));
    }
}
```

## Copy-paste USART1 transmit example

This follows WCH's official [USART_IAP example](https://github.com/openwch/ch32v003/blob/main/EVT/EXAM/USART_IAP/CH32V003_IAP/User/iap.c),
using transmit-only mode at 115200 baud.

```c
#include "debug.h"

int main(void) {
    GPIO_InitTypeDef gpio = {0};
    USART_InitTypeDef usart = {0};

    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOD | RCC_APB2Periph_USART1,
                           ENABLE);
    gpio.GPIO_Pin = GPIO_Pin_5;
    gpio.GPIO_Speed = GPIO_Speed_30MHz;
    gpio.GPIO_Mode = GPIO_Mode_AF_PP;
    GPIO_Init(GPIOD, &gpio);

    usart.USART_BaudRate = 115200;
    usart.USART_WordLength = USART_WordLength_8b;
    usart.USART_StopBits = USART_StopBits_1;
    usart.USART_Parity = USART_Parity_No;
    usart.USART_HardwareFlowControl = USART_HardwareFlowControl_None;
    usart.USART_Mode = USART_Mode_Tx;
    USART_Init(USART1, &usart);
    USART_Cmd(USART1, ENABLE);

    for (;;) {
        while (USART_GetFlagStatus(USART1, USART_FLAG_TXE) == RESET) {}
        USART_SendData(USART1, 'O');
    }
}
```

## Copy-paste SHT31 status read

The [official CH32 Arduino core](https://github.com/openwch/arduino_core_ch32)
supports I2C master mode for CH32V003. Its default `Wire` pins are PC1/SDA and
PC2/SCL. Connect a 3.3 V SHT31-D breakout to those pins and GND.

```cpp
#include <Wire.h>

void setup() {
  Serial.begin(115200);
  Wire.begin();  // CH32V003 default I2C1: PC1/SDA, PC2/SCL

  Wire.beginTransmission(0x44);
  Wire.write(0xF3);  // SHT31 read-status command
  Wire.write(0x2D);
  if (Wire.endTransmission() != 0) {
    Serial.println("SHT31 command failed");
    return;
  }

  delay(1);
  if (Wire.requestFrom((uint8_t)0x44, (uint8_t)2) != 2) {
    Serial.println("SHT31 status read failed");
    return;
  }
  uint16_t status = ((uint16_t)Wire.read() << 8) | Wire.read();
  Serial.print("SHT31 status: 0x");
  Serial.println(status, HEX);
}

void loop() {}
```

## Build and run

```sh
examples/ch32v003-evt/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/onboarding-smoke.yaml \
  --output-dir /tmp/ch32v003-smoke

examples/ch32v003-evt/build-usart-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/usart-smoke.yaml \
  --output-dir /tmp/ch32v003-usart-smoke

examples/ch32v003-evt/build-sht31-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/i2c-smoke.yaml \
  --output-dir /tmp/ch32v003-sht31-smoke

examples/ch32v003-evt/build-systick-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/systick-smoke.yaml \
  --output-dir /tmp/ch32v003-systick-smoke

# Needs riscv64-unknown-elf-gcc + picolibc; set RISCV_PREFIX if not under /usr.
examples/ch32v003-evt/build-sdk-gpio-toggle.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/sdk-gpio-toggle-smoke.yaml \
  --output-dir /tmp/ch32v003-sdk-gpio-toggle

examples/ch32v003-evt/build-sdk-interrupt-examples.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/pfic-smoke.yaml \
  --output-dir /tmp/ch32v003-pfic-smoke
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/sdk-systick-interrupt-smoke.yaml \
  --output-dir /tmp/ch32v003-sdk-systick-interrupt

# ch32fun ELFs are committed; this rebuilds them byte-identically.
examples/ch32v003-evt/build-ch32fun-examples.sh
for t in blink uartdemo systick-irq adc-polled i2c-oled tim2-pwm tim1-pwm tim1-complementary exti \
    input-capture tim2-encoder tim2-pwm-remap spi-oled usart-dma-rx; do
  cargo run -q -p labwired-cli -- test \
    --script examples/ch32v003-evt/ch32fun-$t-smoke.yaml \
    --output-dir /tmp/ch32v003-ch32fun-$t
done
# spi_oled: every latched SSD1306 frame against the example's own Life.
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/ch32fun-spi-oled-smoke.yaml \
  --output-dir /tmp/spi-oled --inspect-bytes-max 4096
python3 examples/ch32v003-evt/check-spi-oled.py /tmp/spi-oled/result.json --min-generations 21
# tim2_pwm's waveform: capture the three pads, then decode them.
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/ch32fun-tim2-pwm-smoke.yaml \
  --output-dir /tmp/ch32v003-tim2-pwm \
  --watch-gpio gpiod:4 --watch-gpio gpiod:3 --watch-gpio gpiod:7
python3 examples/ch32v003-evt/check-tim2-pwm.py /tmp/ch32v003-tim2-pwm/result.json
# tim1_pwm's waveform: CH1N on PD0 and CH4 on PC4.
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/ch32fun-tim1-pwm-smoke.yaml \
  --output-dir /tmp/ch32v003-tim1-pwm \
  --watch-gpio gpiod:0 --watch-gpio gpioc:4
python3 examples/ch32v003-evt/check-tim1-pwm.py /tmp/ch32v003-tim1-pwm/result.json
# tim1_pwm_complementary_outputs: the remapped pair on PC6/PC3 and its dead
# time. The smoke stops at DTG ~15; the committed capture is the same script
# with max_cycles 265000000 / max_steps 400000000, which sweeps DTG 0..255
# and wraps (--full requires that).
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/ch32fun-tim1-complementary-smoke.yaml \
  --output-dir /tmp/ch32v003-tim1-complementary \
  --watch-gpio gpioc:6 --watch-gpio gpioc:3
python3 examples/ch32v003-evt/check-tim1-complementary.py /tmp/ch32v003-tim1-complementary/result.json
# tim2_pwm_remap: the pair on PD4/PD3, then on PC1/PC7 after the remap.
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/ch32fun-tim2-pwm-remap-smoke.yaml \
  --output-dir /tmp/ch32v003-tim2-pwm-remap \
  --watch-gpio gpiod:4 --watch-gpio gpiod:3 --watch-gpio gpioc:1 --watch-gpio gpioc:7
python3 examples/ch32v003-evt/check-tim2-pwm-remap.py /tmp/ch32v003-tim2-pwm-remap/result.json
# exti_pin_change_isr: the PD3 presses and the PC1 blips they cause.
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/ch32fun-exti-smoke.yaml \
  --output-dir /tmp/ch32v003-exti \
  --watch-gpio gpioc:1 --watch-gpio gpiod:3
```

The committed SysTick and SDK-example runs are in
[`evidence/systick-smoke/`](evidence/systick-smoke/) and
[`evidence/sdk-gpio-toggle-smoke/`](evidence/sdk-gpio-toggle-smoke/). The I2C run
is in [`evidence/sht31-smoke/`](evidence/sht31-smoke/), and the earlier GPIO and
USART receipts are alongside it. The committed fixtures are
handwritten assembly without calls or an ABI-dependent stack frame because the
upstream Rust toolchain warns that its RV32E ABI support is incomplete. These
are deterministic local engine runs, not physical-board captures or hosted
`labwired_verify` results.
