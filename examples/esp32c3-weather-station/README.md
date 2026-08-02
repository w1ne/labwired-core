# E-Paper Weather Station (ESP32-C3 + WeAct 2.9″ + GxEPD2 C90c)

A desk weather display you can actually reuse. An ESP32-C3 joins your WiFi,
fetches the current conditions for your city from Open-Meteo — free, no account,
no API key — and paints them on a 2.9″ black/white/red e-ink panel. It re-fetches
on a timer; between refreshes the panel draws no current at all, so the last
reading stays readable even with the board unplugged.

Set what's yours at the top of `src/main.ino`:

```c
static const char *WIFI_SSID = "labwired-ap";      // your WiFi
static const char *WIFI_PASS = "";                 // "" for an open network
static const char *TITLE = "ANDRII'S WEATHER";     // the red line across the top
static const char *CITY = "BUDAPEST";              // just the name
static const unsigned long REFRESH_MINUTES = 30;
```

**No coordinates to look up.** The city name is the whole setting — Open-Meteo's
geocoder turns it into a latitude and longitude on the first fetch, and the
result is kept for the rest of the boot, since a city does not move between
refreshes. `"New York"` and other names with spaces are fine; they're
percent-encoded. A name that matches nothing paints `CITY NOT FOUND` rather
than quietly falling back to 0°,0° in the Gulf of Guinea.

`WIFI_SSID` ships as `labwired-ap` so the lab runs in the simulator out of the
box — change it to your home network before flashing real hardware.

## What lands on the panel

```
ANDRII'S WEATHER                        BUDAPEST
────────────────────────────────────────────────
   \ ; /                              THU 30 JUL
  -- O --      32°C                      HUM 25%
   / ; \                                 HI 35.8
CLEAR                                    LO 23.8
```

Title, temperature and the sun/precipitation in **red**; cloud and text in
black. The icon is drawn from primitives (`draw_sun`, `draw_cloud`, …) rather
than a bitmap, so there is no glyph table to carry and it scales cleanly.
Conditions come from the WMO weather code — `icon_for()` picks the icon,
`code_text()` the label.

**Date, not clock.** With a 30-minute refresh a printed time is stale the moment
it reaches the glass, so the panel shows the weekday and date instead. The
weekday is computed locally with Zeller's congruence — no RTC, no NTP.

### Fitting the layout

Adafruit_GFX wraps overflowing text to the next line by default, which strands
the tail at x=0 — `PARTLY CLOUDY` printed at x=150 is 154px wide on a 296px
panel, so it shed a lone `Y` onto the line below. Two things prevent that here:
`setTextWrap(false)` as a backstop, and a layout sized so nothing needs to clip
— the condition gets its own full-width line (longest label `FREEZING DRIZZLE`
ends at x=181) and the stats column is right-aligned to x=290 (widest entry on
that baseline starts at x=220).

If you lengthen a label or add a field, check the widths rather than eyeballing
them: sum the `xAdvance` values in the font header for your string.

## Refresh rate

Tri-color panels are slow: a full refresh takes ~15 s, and the panel datasheets
ask for at least ~3 minutes between updates. Keep `REFRESH_MINUTES` well above 5.
Upstream data only moves every 15 minutes anyway, so 30 is a good default.

The sketch stays awake on a `millis()` timer rather than deep sleep, which keeps
it simple and observable in the twin. For a battery build, replace the `loop()`
timer with `esp_deep_sleep_start()` after the paint.

## Correct lock (do not break)

| Layer | Value |
|--------|--------|
| **Driver** | `GxEPD2_290_C90c` |
| **Twin / diagram type** | **`ssd1680_tricolor_290`** |
| **Not** | `uc8151d_tricolor_290` (that is the `GxEPD2_290_Z13c` panel) |

Several WeAct panels this size look identical but need different driver opcodes,
so the twin has to match the *driver class* you instantiate — which is a property
of the class, not of the MCU.

Despite the name, `GxEPD2_290_C90c` speaks **SSD1680**. Read
`GxEPD2_290_C90c::_InitDisplay()` in GxEPD2 1.6.0: `0x12` SWRESET, `0x01` driver
output control, `0x11` data entry, `0x3C` border, `0x21`, then `0x22`+`0x20` to
trigger the update, with RAM writes on `0x24`/`0x26`. Captured off the wire, a
C90c build sends:

```
12 01 27 01 00 11 03 3c 05 18 80 21 00 80 44 ...   <- SSD1680
```

A `GxEPD2_290_Z13c` build — the UC8151D panel — sends something else entirely:

```
00 8f 61 80 01 28 50 77 04                          <- UC8151D (PSR, TRES, CDI, PON)
```

`ssd1680_tricolor_290` is written against exactly the 15 commands C90c emits.
Pick `uc8151d_tricolor_290` here and the panel decodes that stream as PWR/LUT/DRF,
never receives a plane, and stays blank — which is precisely what happened to the
labwired-ereader lab until 2026-07-30.

## Pins (weather-station diagram)

| Signal | GPIO |
|--------|------|
| SCK | 4 |
| MOSI | 6 |
| CS | 7 |
| DC | 2 |
| RST | 3 |
| BUSY | 5 |

## Buy / docs

- [WeAct 2.9″ B/W/R (AliExpress)](https://www.aliexpress.com/item/1005004644515880.html)
- [WeAct EpaperModule](https://github.com/WeActStudio/WeActStudio.EpaperModule)
- [Open-Meteo forecast API](https://open-meteo.com/en/docs)

## Build

Arduino-ESP32 + `zinggjm/GxEPD2`. The library is **not** inferred from the
`#include` — you have to declare it, or the build stops at
`GxEPD2_3C.h: No such file or directory`.

Hosted compile — pass it in `lib_deps`:

```json
{ "board": "esp32-c3-supermini", "language": "arduino",
  "entryPath": "src/main.ino", "lib_deps": ["zinggjm/GxEPD2"] }
```

Locally with PlatformIO:

```ini
[env:esp32c3]
platform = espressif32
board = esp32-c3-devkitm-1
framework = arduino
lib_deps = zinggjm/GxEPD2
```

That pulls Adafruit GFX and BusIO as dependencies and builds clean (~780 KB
flash, 60% of a 4 MB part). Or paste `src/main.ino` into the LabWired
weather-station project and flash.
