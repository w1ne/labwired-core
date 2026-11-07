# ESP8684-DevKitM-1-H4X

**Demand rank:** ESP32 is rank 1 in the JetBrains State of Embedded MCU survey
(38%); ESP32-C2 has no separate rank in that survey. This is an exact,
orderable Espressif development board in the highest-demand family, and it is
another ESP32 variant rather than a new vendor or CPU core. The exact
`ESP8684-DevKitM-1-H4X` SKU was in stock when checked; see
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The profile models the board's three discrete, common-anode RGB LED channels:
red on GPIO0, green on GPIO1, and blue on GPIO8. The smoke firmware configures
those pads for GPIO output and pulses each channel low and high. GPIO8 is a
strapping pin, so the firmware uses it after boot. The chip profile includes
only the C2 GPIO output-register subset and the three IO_MUX pads used here;
Wi-Fi, Bluetooth, USB, UART, boot buttons, and other peripherals are outside
this board profile. Register addresses and offsets are from Espressif's
[ESP8684 Technical Reference Manual](https://www.espressif.com/sites/default/files/documentation/esp8684_technical_reference_manual_en.pdf).

## Copy-paste ESP-IDF example

```c
#include "driver/gpio.h"
#include "esp_rom_sys.h"

static const gpio_num_t rgb_channels[] = {
    GPIO_NUM_0, GPIO_NUM_1, GPIO_NUM_8
};

void app_main(void) {
    const uint64_t channel_mask = (1ULL << 0) | (1ULL << 1) | (1ULL << 8);
    gpio_config_t config = {
        .pin_bit_mask = channel_mask,
        .mode = GPIO_MODE_OUTPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    gpio_config(&config);

    for (int i = 0; i < 3; ++i) {
        gpio_set_level(rgb_channels[i], 1); // common-anode LED off
    }

    while (1) {
        for (int i = 0; i < 3; ++i) {
            gpio_set_level(rgb_channels[i], 0); // sink current: channel on
            esp_rom_delay_us(250000);
            gpio_set_level(rgb_channels[i], 1);
            esp_rom_delay_us(250000);
        }
    }
}
```

## Run the simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp8684-devkitm-1-h4x \
  --crate firmware-esp8684-devkitm1-h4x-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp8684-devkitm-1-h4x/onboarding-smoke.yaml \
  --system configs/systems/esp8684-devkitm-1-h4x.yaml \
  --out-dir examples/esp8684-devkitm-1-h4x/evidence
```

The committed evidence records the local simulator smoke and vendor check. It
does not represent a physical-board test. The hosted verifier's current board
catalog does not include ESP8684/C2; see
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).
