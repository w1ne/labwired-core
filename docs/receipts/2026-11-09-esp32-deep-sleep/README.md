# ESP32-DevKitC-32E: stock deep sleep and light sleep with a timer wake-up

**Demand rank:** ESP32 is #1 (38 % of respondents, JetBrains State of Embedded
2025). This is depth work on a part already modelled, not a new part. The
tested system is Espressif's ESP32-DevKitC-32E with its ESP32-WROOM-32E
module. Deep sleep with a timer wake-up is the standard pattern for a
battery-powered ESP32 sensor node (Arduino-ESP32 example
`DeepSleep/TimerWakeUp`).

**Vendor status** (checked live 2026-11-09): Espressif's current
[ESP32-DevKitC V4 guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
lists the exact ESP32-DevKitC-32E / ESP32-WROOM-32E combination. Its current
[module catalog](https://www.espressif.com/products/modules) lists
ESP32-WROOM-32E-N4 and purchase links for DigiKey, Mouser, Amazon US, and
Robu India. Espressif's
[longevity commitment](https://www.espressif.com/en/products/longevity-commitment)
lists ESP32 for at least 15 years from 2016 (through 2031 at minimum). These
current vendor listings establish that the tested board/module remains
available; the longevity page is a supply commitment, not an NRND status field.

## Result

| sketch | before (lane `193e168d6`) | after |
|---|---|---|
| `deep_sleep_timer.cpp` | fault at `0x3FF5_D054` after `sleeping` | `LW_DS_OK`, 2,027,761 steps |
| `light_sleep_timer.cpp` | not run (same `rtc_sleep_init` fault path) | `LW_LS_OK`, 702,155 steps |

Deep sleep, after (`after/deep_sleep_timer.uart.log`):

```
boot=1 cause=0 reset=1      power-on: ESP_SLEEP_WAKEUP_UNDEFINED, ESP_RST_POWERON
sleeping
boot=2 cause=4 reset=8      ESP_SLEEP_WAKEUP_TIMER, ESP_RST_DEEPSLEEP
sleeping
boot=3 cause=4 reset=8
LW_DS_OK
```

`bootCount` is an `RTC_DATA_ATTR` variable. It counts 1, 2, 3, so RTC slow
memory survives the wake-up reset while IRAM and DRAM are reloaded.

## What changed

1. **Radio power-down windows.** `rtc_sleep_init` clears FORCE_PU bits in
   `FE2_TX_INTERP_CTRL`, `FE_GEN_CTRL`, `NRXPD_CTRL` and `BBPD_CTRL` before
   every sleep. FE2 `0x3FF4_5000`, FE `0x3FF4_6000`, NRX `0x3FF5_C000` and
   BB `0x3FF5_D000` are now plain register stubs. No radio is modelled.
2. **RTC_CNTL sleep** (`peripherals/esp32/rtc_cntl.rs`). A write of
   `STATE0.SLEEP_EN` with `TIMER_TRIG` in `WAKEUP_ENA` moves the RTC slow
   counter to the `SLP_TIMER0/1` alarm and sets `WAKEUP_CAUSE = TIMER_TRIG`.
   With `DIG_PWC.DG_WRAP_PD_EN` set (deep sleep), both cores take
   `DEEPSLEEP_RESET` and a wake latch is raised. Otherwise (light sleep)
   `INT_RAW.SLP_WAKEUP` is set, so `rtc_sleep_start`'s poll returns.
   `INT_CLR` now clears `INT_RAW`.
3. **Deep-sleep reboot** (`Machine::capture_esp32_deep_sleep_boot`). The
   `labwired test` ESP32 path records the IRAM/DRAM segment bytes, entry PC
   and stacks after its fast-boot setup. On the wake latch the machine
   writes those bytes back, resets both cores (APP_CPU halted until PRO_CPU
   releases it) and starts at the entry. RTC_CNTL, RTC slow and RTC fast
   memory are not touched. This is what the 2nd-stage bootloader does on
   silicon: it skips the RTC segments after `DEEPSLEEP_RESET`.
4. **ROM `rtc_get_reset_reason`** (`0x4000_81d4`) now decodes
   `RTC_CNTL_RESET_STATE` per core, as the BROM does. Before, it always
   returned POWERON_RESET.

The intermediate build in `before/deep_sleep_timer.txt` is a negative
control: with item 4 missing, the sketch wakes and counts but prints
`LW_DS_FAIL`.

## Not covered

- Wake sources other than the RTC timer (EXT0/EXT1 GPIO, touch, ULP,
  UART). With only those enabled the chip sleeps for ever.
- Sleep duration in CPU time. Only the RTC slow counter jumps to the alarm;
  the cycle counter and `esp_timer` do not advance across the sleep.
- Power figures, the deep-sleep wake stub, and RTC GPIO hold.
- The wasm/playground path and `--rom-boot`. They do not capture a reboot
  state, so a deep sleep there halts both cores.
- ESP32-S2/S3/C3/C6. Their sleep goes through other register blocks
  (and PMU on the C6) and is not modelled.

## Reproduce

From the repository root, build both sketches with the pinned PlatformIO
project, then run the two acceptance scripts:

```sh
pio run --project-dir docs/receipts/2026-11-09-esp32-deep-sleep/sketch
labwired test --script docs/receipts/2026-11-09-esp32-deep-sleep/sketch/deep_sleep_timer.test.yaml --output-dir out/deep-sleep
labwired test --script docs/receipts/2026-11-09-esp32-deep-sleep/sketch/light_sleep_timer.test.yaml --output-dir out/light-sleep
```

The project pins Espressif PlatformIO platform 8.0.0; the recorded build used
Arduino-ESP32 3.3.12. Both scripts use the exact
`configs/systems/esp32-devkitc-32e-gpio-smoke.yaml` board system.
