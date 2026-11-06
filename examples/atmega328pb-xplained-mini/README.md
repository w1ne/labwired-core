# ATmega328PB Xplained Mini

**Demand position:** The survey's ATmega328 row is rank **#2 (30%)**, but it does
not separately measure the PB sibling. The classic ATmega328P is NRND, so this
round uses Microchip's **ATmega328PB**, whose live vendor status is In Production.
This is active sibling-board maintenance in an already supported AVR family, not
a new MCU-family demand pick; the survey share is not attributed to the PB variant.

The focused model boots AVR firmware, blinks the onboard yellow D200 LED on PB5,
and sends `XPLAINED_MINI_OK` through USART0. The twin captures it at a host-console
connector corresponding to the board's CDC UART route; it does not emulate the
mEDBG microcontroller or USB. The user button, QTouch pads, and other headers are
also outside the profile.

## Copy-paste AVR C example

Build for the ATmega328PB with an AVR-GCC toolchain that supports the PB device:

```c
#include <stdint.h>

#define DDRB  (*(volatile uint8_t *)0x24)
#define PORTB (*(volatile uint8_t *)0x25)

int main(void) {
    DDRB |= (1u << 5);
    for (;;) {
        PORTB ^= (1u << 5);  // D200 user LED on PB5
        for (volatile uint16_t i = 0; i < 50000; ++i) {}
    }
}
```

Microchip's live product page reported the part **In Production**, and the exact
Xplained Mini board was listed Active and in stock. See the [availability receipt](evidence/vendor-status.md).

## Run the committed smoke

```sh
scripts/onboarding_smoke.sh \
  --target-id atmega328pb-xplained-mini \
  --firmware-build-script examples/atmega328pb-xplained-mini/ci/build-firmware.sh \
  --script examples/atmega328pb-xplained-mini/onboarding-smoke.yaml \
  --system configs/systems/atmega328pb-xplained-mini.yaml \
  --out-dir examples/atmega328pb-xplained-mini/evidence
```

The evidence is a local simulator smoke, not a physical-board test. The firmware
also drives PB5 while producing the CDC marker; both the UART marker and repeated
LED transitions are asserted.
