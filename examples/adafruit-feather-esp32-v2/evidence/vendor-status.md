# Live vendor status: Adafruit ESP32 Feather V2

Checked 2026-11-04 against Adafruit's live product listing and current
Espressif documentation.

- [Adafruit ESP32 Feather V2, PID 5400](https://www.adafruit.com/product/5400)
  listed the PCB-antenna version at **$19.95**, **41 in stock**, with an
  **Add to Cart** option. The board page identifies the 8 MiB flash / 2 MiB
  PSRAM Feather V2.
- The headered version, [PID 5900](https://www.adafruit.com/product/5900),
  listed at **$20.95**, **25 in stock**, with **Add to Cart**.
- Adafruit's [CH9102F driver guide](https://learn.adafruit.com/how-to-install-drivers-for-wch-usb-to-serial-chips-ch9102f-ch9102)
  says the ESP32 Feather V2 uses the WCH CH9102F USB-to-serial bridge and
  explains that CH9102F is replacing the former CP2104/CP2102N parts. This
  onboarding targets the current Feather V2 revision and does not claim the
  older CP2104/CP2102N bridge variants.
- Espressif's [current ESP32 Series datasheet](https://documentation.espressif.com/esp32_datasheet_en.html)
  (v5.3, live on 2026-11-04) names **ESP32-D0WD-V3** without an NRND/EOL
  designation; it separately marks ESP32-D0WD, ESP32-D0WDQ6, and
  ESP32-D0WDQ6-V3 as NRND and ESP32-D0WDR2-V3 as EOL. LabWired's board catalog
  identifies the Feather V2 MCU as ESP32-D0WD-V3.

The result supports choosing the currently stocked PID 5400 board and its
current ESP32-D0WD-V3/CH9102F revision. The stock statement is time-sensitive
and should be rechecked for later onboarding rounds.
