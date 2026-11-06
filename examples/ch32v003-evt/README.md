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
