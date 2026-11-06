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
`ch32v003f4p6-ch32fun-smoke` profile runs seven of its examples unmodified,
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
| `tim2_pwm` | TIM2 PWM on CH1 (PD4), CH2 (PD3) and CH4 (PD7), with an LED on each; debug-printf banner and the TIM2 setup left behind (ATRLR, CEN/ARPE, CHCTLR1/2, CCER). [`check-tim2-pwm.py`](check-tim2-pwm.py) decodes the waveform of a `--watch-gpio` run, committed as [`pwm-capture.json.gz`](evidence/ch32fun-tim2-pwm-smoke/pwm-capture.json.gz) with its output in [`pwm-check.log`](evidence/ch32fun-tim2-pwm-smoke/pwm-check.log) | 11,546 whole 256-cycle periods (187.5 kHz); CH1 low for `count`/256 and CH2 low, CH4 high for `(count + 128) & 255`/256, through CH2's 255 → 0 wrap; `count` steps by 1 every 937.5 periods, i.e. `Delay_Ms(5)` |
| `exti_pin_change_isr` | a button on PD3 is pressed, released and pressed again; the EXTI7_0 handler blips PC1. The smoke checks PC1 edges and the EXTI/AFIO/PFIC state left behind; the `--watch-gpio` run is committed as [`pc1-capture.json`](evidence/ch32fun-exti-smoke/pc1-capture.json) | PD3 rises at 200,001 and 600,001 and PC1 blips at 200,007-200,009 and 600,007-600,009: exactly 4 PC1 edges, none at the release (falling edge, FTENR clear) at 400,001 |

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
  peripheral the other ch32fun examples need (TIM1, SPI, DMA, OPA,
  watchdogs, standby).
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
- On ADC1: sample time (`SAMPTR`) does not change the conversion time, and
  the injected group, scan sequences (only RSQR3's first channel converts),
  external triggers, the analog watchdog, DMA, the internal V_REFINT/V_CAL
  channels and calibration offsets are not modeled. Codes are ideal (no
  offset, gain or noise error) against a 3.3 V reference.
- On TIM2: the AFIO `PCFR1` TIM2 remaps (stored only, so a remapped
  output stays on its reset pad), the `RCC_APB1PRSTR` reset, PD7's NRST
  option byte (the pad is treated as GPIO, as after `minichlink -d`),
  encoder and slave modes, DMA and TIM2 interrupts are not modeled. The
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
for t in blink uartdemo systick-irq adc-polled i2c-oled tim2-pwm exti; do
  cargo run -q -p labwired-cli -- test \
    --script examples/ch32v003-evt/ch32fun-$t-smoke.yaml \
    --output-dir /tmp/ch32v003-ch32fun-$t
done
# tim2_pwm's waveform: capture the three pads, then decode them.
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/ch32fun-tim2-pwm-smoke.yaml \
  --output-dir /tmp/ch32v003-tim2-pwm \
  --watch-gpio gpiod:4 --watch-gpio gpiod:3 --watch-gpio gpiod:7
python3 examples/ch32v003-evt/check-tim2-pwm.py /tmp/ch32v003-tim2-pwm/result.json
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
