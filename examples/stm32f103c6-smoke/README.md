# STM32F103C6T6A low-density smoke

**Demand position:** the STM32F103 family is measured MCU rank 3 (17%). This
work validates the exact low-density C6 profile already in the catalog. It is
one exact variant within an existing family, not a new demand pick.

ST currently lists the STM32F103C6T6A as Active and In Stock. This example is a
customer minimum-system circuit built around that MCU: USART2 on PA2/PA3 sends
a banner, and PA5 drives an off-board LED. It is a part-level model profile,
not an evaluation-board descriptor.

## Connections

| Signal | STM32F103C6 pin | Example connection |
| --- | --- | --- |
| USART2 TX | PA2 | 3.3 V USB-serial adapter RX |
| USART2 RX | PA3 | 3.3 V USB-serial adapter TX (unused by this smoke) |
| LED output | PA5 | LED and current-limiting resistor to GND |
| Ground | VSS | Adapter and LED circuit ground |

The simulator attaches a logic-only LED observer to PA5. It does not model LED
current, brightness, the adapter, board power, or physical wiring.

## Copy-paste Arduino example

For an F103C6 carrier configured with STM32duino (the exact board definition
depends on the carrier):

```cpp
#include <Arduino.h>

void setup() {
  pinMode(PA5, OUTPUT);
  Serial2.begin(115200);  // USART2: PA2 TX, PA3 RX
  Serial2.println("STM32F103C6_LOW_DENSITY_OK");
}

void loop() {
  digitalWrite(PA5, HIGH);
  delay(500);
  digitalWrite(PA5, LOW);
  delay(500);
}
```

The simulator smoke is the register-level Rust firmware at
[`crates/firmware-stm32f103-lowdensity-smoke/src/main.rs`](../../crates/firmware-stm32f103-lowdensity-smoke/src/main.rs).
It writes a banner through USART2 and toggles PA5. The low-density debug-port
IDCODE is `0x10000412` per RM0008 §31.6.1, but ST errata ES0348 §2.2.2 says
user-mode firmware reads `DBGMCU_IDCODE` as zero. The C6 CPU-visible model
therefore returns zero; this smoke does not probe the debug-only IDCODE.

## Run the simulator smoke

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f103c6-low-density \
  --crate firmware-stm32f103-lowdensity-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103c6-smoke/io-smoke.yaml \
  --system examples/stm32f103c6-smoke/system.yaml \
  --out-dir examples/stm32f103c6-smoke/evidence
```

The committed evidence checks USART2 output, PA5 transitions, and bounded stop.
This is model verification, not a physical C6 capture.

## Model boundary

The C6 descriptor uses the low-density peripheral instance set: SPI1, I2C1,
USART1/2, TIM1-3, ADC1/2, and DMA1. It leaves out C8-only SPI2, I2C2, USART3,
and TIM4 windows. The USB device and backup-register windows are explicit
stubs; this smoke does not touch them. The exact flash, SRAM, package, pin count,
ADC count, and debug-port IDCODE come from ST DS5792/RM0008. ES0348's restriction
on user-mode DBGMCU reads is reflected in the C6 configuration. No physical
board or silicon test has been run.

See [`vendor-status.md`](vendor-status.md) for the live lifecycle and stock
check.
