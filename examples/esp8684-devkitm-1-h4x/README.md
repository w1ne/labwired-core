# ESP8684-DevKitM-1-H4X

**Demand rank:** ESP32 is rank 1 in the JetBrains State of Embedded MCU survey
(38%); ESP32-C2 has no separate rank in that survey. This is an exact,
orderable Espressif development board in the highest-demand family, and it is
another ESP32 variant rather than a new vendor or CPU core. This round is
maintenance/depth work on the existing board model, not a newly onboarded SKU.
The exact `ESP8684-DevKitM-1-H4X` SKU is listed by Espressif and was in stock
at Mouser when checked; see
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The profile models the board's three discrete, common-anode RGB LED channels
(GPIO0, GPIO1, GPIO8) and an external potentiometer wired to GPIO2 / ADC1_CH2.
The smoke pulses the RGB channels and reads the ADC1 one-shot result. Its
1.65 V midpoint fixture produces raw code 2047 with channel tag 2. The C2 ADC
surface reuses the C3 one-shot behavior model because the ESP8684 TRM specifies
the same one-shot offsets and handshake. The model does not claim ADC
calibration, continuous conversion, DMA, threshold monitoring, or temperature
sensing. The reused model's reset seeds were captured from C3 silicon, not
ESP8684 silicon; this smoke exercises the conversion handshake and channel
result only. Its analog input mapping is a deterministic linear 0–3.3 V model,
not an ADC calibration claim. GPIO8 is a strapping pin, so firmware uses it
after the chip has booted. Wi-Fi, Bluetooth, USB, UART, boot buttons, and other
peripherals remain outside this narrow profile. Register addresses and offsets
are from Espressif's
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

## Copy-paste ESP-IDF ADC example

Connect a 10 kΩ potentiometer between 3V3 and GND, with its wiper on GPIO2.
Keep the wiper within the board's 0–3.3 V supply range.

```c
#include "esp_check.h"
#include "esp_adc/adc_oneshot.h"

void app_main(void) {
    adc_oneshot_unit_handle_t adc1;
    const adc_oneshot_unit_init_cfg_t unit = {
        .unit_id = ADC_UNIT_1,
    };
    ESP_ERROR_CHECK(adc_oneshot_new_unit(&unit, &adc1));

    const adc_oneshot_chan_cfg_t channel = {
        .atten = ADC_ATTEN_DB_12,
        .bitwidth = ADC_BITWIDTH_DEFAULT,
    };
    ESP_ERROR_CHECK(adc_oneshot_config_channel(adc1, ADC_CHANNEL_2, &channel));

    int raw = 0;
    ESP_ERROR_CHECK(adc_oneshot_read(adc1, ADC_CHANNEL_2, &raw));
    // Use `raw` as the 12-bit ADC1_CH2 reading from GPIO2.
}
```

The API shape follows Espressif's current [ESP32-C2 ADC oneshot guide](https://docs.espressif.com/projects/esp-idf/en/stable/esp32c2/api-reference/peripherals/adc/adc_oneshot.html).

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
