# ESP8684-DevKitM-1-H4X live vendor status

Checked **2026-11-17 UTC** for the exact `ESP8684-DevKitM-1-H4X` board.

- Espressif's [current ESP8684-DevKitM-1 v1.1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c2/esp8684-devkitm-1/user_guide.html)
  identifies the H4X order code, ESP8684-MINI-1 module, and 4 MB flash. It
  marks the older `ESP8684-DevKitM-1-H4` as NRND; the H4X row is listed without
  that lifecycle label.
- Espressif's [live DevKits catalog](https://www.espressif.com/en/products/devkits/esp32-devkitm-1/overview)
  lists the exact H4X variant and buy links for AliExpress, DigiKey, Mouser,
  and Amazon.
- Mouser's [exact H4X listing](https://www.mouser.hk/en/ProductDetail/Espressif-Systems/ESP8684-DevKitM-1-H4X)
  reported **35 in stock**, immediately dispatchable, with quantity 1 orderable.
- Espressif's [longevity commitment](https://www.espressif.com/en/products/longevity-commitment)
  lists ESP32-C2 for at least 12 years from 2022-01-01 (through at least
  2034-01-01). The page's NRND section does not list ESP32-C2.

This evidence applies to the H4X board and ESP32-C2 family only. The H4 sibling
is NRND and is explicitly excluded.

## 2026-11-18 lifecycle and availability recheck

- Espressif's [live product selector API](https://products.espressif.com/api/user/products)
  returned `ESP8684H4X` and `ESP8684-MINI-1-H4X` as **Mass Production**. It
  returned the superseded `ESP8684H4` as **Replaced** by `ESP8684H4X`.
- Mouser's [ESP8684H4X listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP8684H4X)
  showed silicon inventory. The quantity surfaced in the current catalog
  result was 4,936; exact quantities vary by region and over time.
- The exact [ESP8684-DevKitM-1-H4X board](https://www.mouser.com/ProductDetail/Espressif-Systems/ESP8684-DevKitM-1-H4X)
  remains orderable. Mouser's recent EU catalog result showed 35 in stock,
  while its US listing reported non-stocked with an eight-week factory lead
  time. This is a regional stock difference, not an obsolete lifecycle status.
