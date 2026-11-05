# WCH CH32V003F4P6 on the CH32V003EVT

**Demand rank:** Tier C qualitative vendor bet, not a measured MCU rank in the
JetBrains survey. The measured ranks remain ESP32 (38%), ATmega328 (30%),
STM32F103 (17%), other STM32 (13%), and RP2040 (12%). This round adds WCH's
CH32V003F4P6 as the first WCH/QingKe family profile in the engine.

## What this profile runs

The narrow system boots RV32EC firmware from the CH32V003's 16 KiB flash and
2 KiB SRAM map, stays at the reset 24 MHz HSI clock, enables GPIOD through
APB2PCENR, configures PD0 as push-pull output, and toggles it. The simulation
asserts at least eight output edges. The clock tree, PLL, WCH SDI, UART, I2C,
SPI, ADC, DMA, timers, SysTick, interrupts, power modes, flash controller,
and other GPIO ports are outside this profile. Supply and reset sequencing
are not modeled: the current run assumes an ideal VDD rail, and stack analysis
is unsupported for this architecture.

The engine reuses its STM32F1-shaped GPIO implementation for the matching
CH32 register offsets and BSHR set/reset semantics, exposed through a
`ch32v003` compatibility alias. The RCC model's `ch32v003` alias resolves only
the APB2 enable register at offset `0x18`; on CH32V003 that register is named
`APB2PCENR`. This is a deliberately narrow mapping, not a claim that the WCH
clock tree is STM32F1.

For a physical smoke, connect an LED and current-limiting resistor between PD0
and GND. The vendor manual says the EVT board's LEDs connect through its P4
LED-row pins; use the board schematic to connect that test circuit to PD0. This
model's `external_led_pd0` represents the test connection used by the smoke,
not a factory-installed PD0 LED.

## Copy-paste WCH SDK example

In the WCH CH32V00x EVT SDK's `GPIO_Toggle` project, this uses the same WCH
GPIO and clock APIs as the official PD0 push-pull example. The SDK supplies
`debug.h`, startup code, and the delay helpers.

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

## Rebuild and run the smoke

```sh
examples/ch32v003-evt/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/onboarding-smoke.yaml \
  --output-dir /tmp/ch32v003-smoke
```

The committed run evidence is in `evidence/`. The upstream Rust toolchain warns
that its RV32E ABI support is incomplete; this fixture is handwritten assembly
without calls or an ABI-dependent stack frame. The smoke is a deterministic
engine run; it is not a physical-board test or a hosted `labwired_verify`
result.
