# Raspberry Pi Pico 1 (RP2040)

**Demand position:** RP2040 ranks #5 at 12% in the MCU-use survey. This is a
low-cost maintenance and peripheral-coverage follow-up to the existing Pico
board, not a new-demand pick.

This board profile reuses the RP2040 chip model and binds its onboard LED to
GP25. The smoke builds `firmware-rp2040-demo`, which configures UART0 on GP0,
reads the on-chip temperature-sensor input through the ADC, then selects SIO
for GP25 and toggles the LED through the modeled GPIO output registers. The
test asserts UART output from both firmware paths and at least eight GP25
edges. The ADC smoke checks that a non-zero conversion result reaches the
result register; it does not claim a calibrated temperature measurement.

## Copy-paste Pico SDK example

Enable USB stdio in the Pico SDK project, then read the internal temperature
sensor with:

```c
#include <stdio.h>
#include <stdint.h>
#include "pico/stdlib.h"
#include "hardware/adc.h"

int main(void) {
    stdio_init_all();
    adc_init();
    adc_set_temp_sensor_enabled(true);
    sleep_ms(1);
    adc_select_input(4);

    const uint16_t raw = adc_read();
    printf("RP2040_ADC_TEMP raw=%u\n", raw);
    for (;;) tight_loop_contents();
}
```

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id pico \
  --crate firmware-rp2040-demo \
  --target thumbv6m-none-eabi \
  --script examples/pico/io-smoke.yaml \
  --system configs/systems/rp2040-pico.yaml \
  --out-dir examples/pico/evidence
```

Committed build and simulation logs are in [`evidence/`](evidence/).

## Vendor status

Checked live on 2026-11-06. Raspberry Pi's [Pico product page](https://www.raspberrypi.com/products/raspberry-pi-pico/)
lists Pico boards as **available from $4** and says the Pico 1 series will
remain in production until at least **January 2036**. The captured page text is
recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Scope

- The smoke exercises the UART0 PL011 model and GPIO through IO_BANK0 and SIO.
- The ADC check exercises the modeled on-die temperature-sensor channel. It
  does not test an external analog input connected to the board.
- UART bytes are captured from the UART model. This does not assert USB CDC,
  USB boot/flash, or a physical serial connection.
- The RP2040 descriptor still has a TBMAN zero-answering stub. The smoke does
  not access it. The other board peripherals are outside this smoke's claims.
- This is a local engine simulation. No physical Pico capture is recorded.
