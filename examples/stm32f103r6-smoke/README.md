# STM32F103R6T6A low-density smoke

**Demand position:** STM32F103 is MCU rank #3 at 17% in the measured survey.
This adds exact R6 validation within that high-demand family; it is variant
depth, not a new silicon or board demand pick. ST currently lists the exact
STM32F103R6T6A as Active and In stock; see [`vendor-status.md`](vendor-status.md).

This is a customer minimum-system circuit around the STM32F103R6 in LQFP64,
not a development-board descriptor. The local simulator smoke runs a
register-level image against the exact R6 geometry, sends a marker through
USART2 on PA2/PA3, and toggles PA5. The smoke asserts UART output, GPIO edges,
and a bounded stop.

## Connections

| Signal | STM32F103R6 pin | Example connection |
| --- | --- | --- |
| USART2 TX | PA2 | 3.3 V USB-serial adapter RX |
| USART2 RX | PA3 | 3.3 V USB-serial adapter TX (unused by this smoke) |
| LED output | PA5 | LED and current-limiting resistor to GND |
| Ground | VSS | Adapter and LED circuit ground |

The simulator attaches a logic-only LED observer to PA5. It does not model LED
current, brightness, the adapter, board power, or physical wiring.

## Copy-paste Arduino example

For an R6 carrier configured with STM32duino (the exact board definition
depends on the carrier):

```cpp
#include <Arduino.h>

void setup() {
  pinMode(PA5, OUTPUT);
  Serial2.begin(115200);  // USART2: PA2 TX, PA3 RX
  Serial2.println("STM32F103R6_LOW_DENSITY_OK");
}

void loop() {
  digitalWrite(PA5, HIGH);
  delay(500);
  digitalWrite(PA5, LOW);
  delay(500);
}
```

## Run the simulator smoke

```sh
examples/stm32f103r6-smoke/ci/test.sh
```

Committed evidence is in [`evidence/`](evidence/). This is local simulator
evidence, not a hosted `labwired_verify` verdict or a physical-board test.

## Model boundary

The R6 descriptor models the low-density peripheral instances: SPI1, I2C1,
USART1/2, TIM1-3, ADC1/2, and DMA1. It omits the medium-density-only SPI2,
I2C2, USART3, and TIM4 windows. USB and backup registers remain explicit
stubs; this smoke does not touch them. The exact flash, SRAM, LQFP64 package,
pin count, ADC count, and orderable suffixes come from ST DS5792. The shared
STM32F103 register models have not been independently captured on an R6.

See [`vendor-status.md`](vendor-status.md) for the live lifecycle and stock
check.
