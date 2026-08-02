// E-Paper Weather Station — ESP32-C3 + WeAct 2.9" tri-color e-paper
//
// Joins your WiFi, pulls the current conditions for your city from Open-Meteo
// (free, no account, no API key), paints them on the panel, and repeats every
// REFRESH_MINUTES. Between refreshes the display draws no current at all — the
// last reading stays on the glass even if you unplug the board.
//
// Everything you need to change is in the EDIT ME block below.
//
// Uses the SAME proven display stack as labwired-ereader (works on glass + twin):
//   GxEPD2_290_C90c  +  diagram part type ssd1680_tricolor_290
//
// Despite the "C90c" name, this driver class speaks SSD1680: 0x12 SWRESET,
// 0x01 driver output control, 0x11 data entry, 0x3C border, then 0x22+0x20 to
// trigger, with RAM writes on 0x24/0x26. Lock the twin to
// uc8151d_tricolor_290 (which is the GxEPD2_290_Z13c panel) and it decodes
// that stream as PWR/LUT/DRF, never receives a plane, and stays blank. See
// the "Correct lock" table in README.md for the captured command streams.
//
// Panel (buy): WeAct Studio 2.9" B/W/R
//   https://www.aliexpress.com/item/1005004644515880.html
// Module docs: https://github.com/WeActStudio/WeActStudio.EpaperModule
//
// Pins match the LabWired weather-station diagram (ESP32-C3 SuperMini):
//   SCK=4  MOSI=6  CS=7  DC=2  RST=3  BUSY=5
//
// Flow: paint boot -> join WiFi -> GET api.open-meteo.com -> paint weather
// (or offline card), then re-fetch on a timer. Serial marker "PANEL UPDATED"
// after each full refresh.
#include <WiFi.h>
#include <SPI.h>
#include <GxEPD2_3C.h>
#include <Fonts/FreeSansBold9pt7b.h>
#include <Fonts/FreeSans9pt7b.h>
#include <Fonts/FreeSansBold12pt7b.h>
#include <Fonts/FreeSansBold24pt7b.h>

// ===================== EDIT ME =====================

// Your WiFi. Leave WIFI_PASS as "" for an open network.
// "labwired-ap" is the simulator's access point — change it to your home WiFi
// before flashing real hardware.
static const char *WIFI_SSID = "labwired-ap";
static const char *WIFI_PASS = "";

// Shown in red across the top. Make it yours.
static const char *TITLE = "ANDRII'S WEATHER";

// Your city. That is the whole setting — the coordinates are looked up for you
// on the first fetch, so there is no latitude/longitude to go and find.
// "Budapest", "New York", "Kyiv" all work; spaces are fine.
static const char *CITY = "BUDAPEST";

// How often to re-fetch and repaint.
// Tri-color panels take ~15 s per full refresh and their datasheets ask for at
// least ~3 minutes between updates, so keep this comfortably above 5.
static const unsigned long REFRESH_MINUTES = 30;

// =================== END EDIT ME ===================

// ---- Pins (diagram: mcu -> ep) ----
static const int PIN_SCK = 4;
static const int PIN_MOSI = 6;
static const int PIN_CS = 7;
static const int PIN_DC = 2;
static const int PIN_RST = 3;
static const int PIN_BUSY = 5;

static const char *API_HOST = "api.open-meteo.com";
static const char *GEO_HOST = "geocoding-api.open-meteo.com";

// Coordinates for CITY, resolved once on the first successful fetch and kept
// for the life of the boot. Geocoding a name that has not changed on every
// refresh would be a request per half hour for an answer that never moves.
static float g_lat = NAN;
static float g_lon = NAN;

// Panel is 296x128 in landscape (setRotation(1)). Keep every drawn run inside
// RIGHT_EDGE: Adafruit_GFX wraps overflowing text to the next line by default,
// which strands the tail characters at x=0 — "PARTLY CLOUDY" printed at x=150
// is 154px wide, so it shed a lone "Y" onto the line below. setTextWrap(false)
// in setup() is the backstop; the layout below is sized so nothing has to clip,
// and the stats column is right-aligned so the widest line still fits.
static const int16_t RIGHT_EDGE = 290;

// C90c class = SSD1680 command stream on the wire -> twin must be ssd1680_tricolor_290
GxEPD2_3C<GxEPD2_290_C90c, GxEPD2_290_C90c::HEIGHT> display(
    GxEPD2_290_C90c(/*CS=*/PIN_CS, /*DC=*/PIN_DC, /*RST=*/PIN_RST, /*BUSY=*/PIN_BUSY));

struct Weather {
  float temp = NAN;
  float hi = NAN;
  float lo = NAN;
  int humidity = -1;
  int code = -1;
  int year = 0, month = 0, day = 0;
};

static void panel_updated(const char *why) {
  Serial.print("PANEL UPDATED");
  if (why && why[0]) {
    Serial.print(" (");
    Serial.print(why);
    Serial.print(")");
  }
  Serial.println();
}

// WMO weather interpretation codes -> short label.
// https://open-meteo.com/en/docs (see "Weather variable documentation")
static const char *code_text(int code) {
  switch (code) {
    case 0: return "CLEAR";
    case 1: return "MAINLY CLEAR";
    case 2: return "PARTLY CLOUDY";
    case 3: return "OVERCAST";
    case 45: case 48: return "FOG";
    case 51: case 53: case 55: return "DRIZZLE";
    case 56: case 57: return "FREEZING DRIZZLE";
    case 61: case 63: case 65: return "RAIN";
    case 66: case 67: return "FREEZING RAIN";
    case 71: case 73: case 75: return "SNOW";
    case 77: return "SNOW GRAINS";
    case 80: case 81: case 82: return "SHOWERS";
    case 85: case 86: return "SNOW SHOWERS";
    case 95: return "THUNDERSTORM";
    case 96: case 99: return "THUNDER + HAIL";
    default: return "";
  }
}

// --- Weather icons ------------------------------------------------------
// Drawn from primitives rather than bitmaps: nothing to carry but the code,
// and it scales cleanly. Sun and precipitation go in RED, cloud in BLACK —
// which is what makes a tri-color panel worth having.
//
// Plain ints, not an enum: the Arduino preprocessor hoists auto-generated
// prototypes above everything in the .ino, so a custom type used in a function
// signature is not yet declared when those prototypes are compiled.
static const int ICON_SUN = 0;
static const int ICON_SUN_CLOUD = 1;
static const int ICON_CLOUD = 2;
static const int ICON_FOG = 3;
static const int ICON_RAIN = 4;
static const int ICON_SNOW = 5;
static const int ICON_STORM = 6;

static int icon_for(int code) {
  switch (code) {
    case 0: return ICON_SUN;
    case 1: case 2: return ICON_SUN_CLOUD;
    case 3: return ICON_CLOUD;
    case 45: case 48: return ICON_FOG;
    case 51: case 53: case 55: case 56: case 57:
    case 61: case 63: case 65: case 66: case 67:
    case 80: case 81: case 82: return ICON_RAIN;
    case 71: case 73: case 75: case 77: case 85: case 86: return ICON_SNOW;
    case 95: case 96: case 99: return ICON_STORM;
    default: return ICON_CLOUD;
  }
}

static void draw_sun(int cx, int cy, int r, uint16_t color) {
  display.fillCircle(cx, cy, r, color);
  for (int i = 0; i < 8; i++) {
    float a = i * (float)PI / 4.0f;
    int x0 = cx + (int)((r + 3) * cosf(a)), y0 = cy + (int)((r + 3) * sinf(a));
    int x1 = cx + (int)((r + 8) * cosf(a)), y1 = cy + (int)((r + 8) * sinf(a));
    display.drawLine(x0, y0, x1, y1, color);
  }
}

static void draw_cloud(int cx, int cy, int w, uint16_t color) {
  int r = w / 4;
  display.fillCircle(cx - r, cy, r, color);
  display.fillCircle(cx + r, cy, r, color);
  display.fillCircle(cx, cy - r / 2, (r * 5) / 4, color);
  display.fillRect(cx - r, cy, 2 * r, r + 1, color);
}

// Three slanted strokes under the cloud.
static void draw_rain(int cx, int cy, uint16_t color) {
  for (int i = -1; i <= 1; i++) {
    int x = cx + i * 12;
    display.drawLine(x, cy, x - 4, cy + 10, color);
  }
}

static void draw_snow(int cx, int cy, uint16_t color) {
  for (int i = -1; i <= 1; i++) {
    int x = cx + i * 12, y = cy + 5;
    display.drawLine(x - 3, y, x + 3, y, color);
    display.drawLine(x, y - 3, x, y + 3, color);
    display.drawLine(x - 2, y - 2, x + 2, y + 2, color);
    display.drawLine(x - 2, y + 2, x + 2, y - 2, color);
  }
}

static void draw_bolt(int cx, int cy, uint16_t color) {
  display.fillTriangle(cx + 4, cy - 2, cx - 6, cy + 12, cx + 1, cy + 12, color);
  display.fillTriangle(cx + 6, cy + 6, cx - 2, cy + 20, cx + 3, cy + 7, color);
}

static void draw_icon(int kind, int cx, int cy) {
  switch (kind) {
    case ICON_SUN:
      draw_sun(cx, cy, 16, GxEPD_RED);
      break;
    case ICON_SUN_CLOUD:
      draw_sun(cx + 12, cy - 12, 10, GxEPD_RED);
      draw_cloud(cx - 4, cy + 8, 44, GxEPD_BLACK);
      break;
    case ICON_CLOUD:
      draw_cloud(cx, cy, 52, GxEPD_BLACK);
      break;
    case ICON_FOG:
      draw_cloud(cx, cy - 6, 48, GxEPD_BLACK);
      for (int i = 0; i < 3; i++) {
        int y = cy + 12 + i * 6;
        display.drawLine(cx - 20, y, cx + 20, y, GxEPD_BLACK);
      }
      break;
    case ICON_RAIN:
      draw_cloud(cx, cy - 8, 48, GxEPD_BLACK);
      draw_rain(cx, cy + 10, GxEPD_RED);
      break;
    case ICON_SNOW:
      draw_cloud(cx, cy - 8, 48, GxEPD_BLACK);
      draw_snow(cx, cy + 8, GxEPD_BLACK);
      break;
    case ICON_STORM:
      draw_cloud(cx, cy - 8, 48, GxEPD_BLACK);
      draw_bolt(cx, cy + 6, GxEPD_RED);
      break;
  }
}

// --- Text helpers -------------------------------------------------------

static void print_at(int16_t x, int16_t y, const char *s) {
  display.setCursor(x, y);
  display.print(s);
}

/** Draw `s` so its right edge lands on RIGHT_EDGE. */
static void print_right(int16_t y, const char *s) {
  int16_t bx, by;
  uint16_t bw, bh;
  display.getTextBounds(s, 0, 0, &bx, &by, &bw, &bh);
  print_at(RIGHT_EDGE - bw, y, s);
}

// The Free* fonts only carry ASCII 0x20-0x7E, so there is no degree glyph.
// Draw the ring by hand, then the unit letter.
static void print_degree_c(int16_t x, int16_t y, uint8_t radius, uint16_t color) {
  display.drawCircle(x + radius, y + radius, radius, color);
  display.setCursor(x + 2 * radius + 3, y + 2 * radius + 2);
  display.print("C");
}

// Zeller's congruence -> 0=Sat, 1=Sun, 2=Mon ... 6=Fri
static const char *weekday_name(int y, int m, int d) {
  static const char *NAMES[7] = {"SAT", "SUN", "MON", "TUE", "WED", "THU", "FRI"};
  if (m < 3) { m += 12; y -= 1; }
  int K = y % 100, J = y / 100;
  int h = (d + (13 * (m + 1)) / 5 + K + K / 4 + J / 4 + 5 * J) % 7;
  return NAMES[(h + 7) % 7];
}

static const char *month_name(int m) {
  static const char *NAMES[12] = {"JAN", "FEB", "MAR", "APR", "MAY", "JUN",
                                  "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"};
  return (m >= 1 && m <= 12) ? NAMES[m - 1] : "";
}

static void draw_header() {
  display.setTextColor(GxEPD_RED);
  display.setFont(&FreeSansBold9pt7b);
  print_at(6, 18, TITLE);
  display.setTextColor(GxEPD_BLACK);
  display.setFont(&FreeSans9pt7b);
  print_right(18, CITY);
  display.drawFastHLine(6, 24, RIGHT_EDGE - 6, GxEPD_RED);
}

static void draw_boot(const char *status) {
  display.setFullWindow();
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);
    draw_header();
    display.setTextColor(GxEPD_BLACK);
    display.setFont(&FreeSans9pt7b);
    print_at(6, 52, "E-PAPER . ESP32-C3");
    print_at(6, 76, status ? status : "...");
  } while (display.nextPage());
  panel_updated(status);
}

static void draw_offline(const char *why) {
  display.setFullWindow();
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);
    draw_header();
    display.setFont(&FreeSansBold12pt7b);
    display.setTextColor(GxEPD_RED);
    print_at(6, 60, "OFFLINE");
    display.setTextColor(GxEPD_BLACK);
    display.setFont(&FreeSans9pt7b);
    print_at(6, 88, why ? why : "NO LINK");
  } while (display.nextPage());
  panel_updated("offline");
}

static void draw_weather(const Weather &w) {
  char line[48];
  display.setFullWindow();
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);
    draw_header();

    // Condition icon, left.
    draw_icon(icon_for(w.code), 44, 62);

    // Big current temperature, centre-left.
    display.setTextColor(GxEPD_RED);
    display.setFont(&FreeSansBold24pt7b);
    snprintf(line, sizeof line, "%d", (int)lroundf(w.temp));
    print_at(84, 86, line);
    int16_t bx, by;
    uint16_t bw, bh;
    display.getTextBounds(line, 84, 86, &bx, &by, &bw, &bh);
    display.setFont(&FreeSansBold12pt7b);
    print_degree_c(84 + bw + 6, 52, 4, GxEPD_RED);

    // Stats column, right-aligned to RIGHT_EDGE. Right-aligning (rather than
    // left-aligning at a fixed column) both keeps the numbers on a clean edge
    // and makes the widest line — "WED 31 DEC", 108px — fit without hand
    // measurement.
    display.setTextColor(GxEPD_BLACK);
    display.setFont(&FreeSans9pt7b);
    if (w.year) {
      snprintf(line, sizeof line, "%s %d %s", weekday_name(w.year, w.month, w.day),
               w.day, month_name(w.month));
      print_right(46, line);
    }
    if (w.humidity >= 0) {
      snprintf(line, sizeof line, "HUM %d%%", w.humidity);
      print_right(70, line);
    }
    if (!isnan(w.hi)) {
      snprintf(line, sizeof line, "HI %.1f", w.hi);
      print_right(94, line);
    }
    if (!isnan(w.lo)) {
      snprintf(line, sizeof line, "LO %.1f", w.lo);
      print_right(118, line);
    }

    // Condition text gets its own full-width line — the longest label
    // ("FREEZING DRIZZLE", 175px) ends at x=181, clear of the stats column,
    // whose widest entry on this baseline starts at x=220.
    display.setTextColor(GxEPD_BLACK);
    display.setFont(&FreeSansBold9pt7b);
    print_at(6, 118, code_text(w.code));
  } while (display.nextPage());
  panel_updated("weather");
}

// --- Tiny JSON scraping -------------------------------------------------
// Open-Meteo repeats every key once under "*_units" (as a string) before the
// real numeric value, so every lookup must start at the section marker.

static int section_start(const String &b, const char *section) {
  return b.indexOf(String("\"") + section + "\":{");
}

// Value of `key` after `from`. Skips a leading '[' so daily arrays work too.
static const char *value_at(const String &b, const char *key, int from, int *found) {
  *found = -1;
  if (from < 0) return nullptr;
  String needle = String("\"") + key + "\":";
  int i = b.indexOf(needle, from);
  if (i < 0) return nullptr;
  i += needle.length();
  if (i < (int)b.length() && b[i] == '[') i++;
  *found = i;
  return b.c_str() + i;
}

static float json_float(const String &b, const char *key, int from) {
  int at;
  const char *p = value_at(b, key, from, &at);
  return p ? atof(p) : NAN;
}

static int json_int(const String &b, const char *key, int from) {
  int at;
  const char *p = value_at(b, key, from, &at);
  return p ? atoi(p) : -1;
}

// "2026-07-30T22:45" -> y/m/d. The clock is deliberately dropped: with a
// half-hour refresh a printed time is stale the moment it lands on the glass.
static bool json_date(const String &b, int from, int *y, int *m, int *d) {
  int at;
  if (!value_at(b, "time", from, &at)) return false;
  int q1 = b.indexOf('"', at);
  if (q1 < 0) return false;
  int q2 = b.indexOf('"', q1 + 1);
  if (q2 < 0) return false;
  String t = b.substring(q1 + 1, q2);
  if (t.length() < 10) return false;
  *y = t.substring(0, 4).toInt();
  *m = t.substring(5, 7).toInt();
  *d = t.substring(8, 10).toInt();
  return *y > 0 && *m >= 1 && *m <= 12 && *d >= 1 && *d <= 31;
}

// Percent-encode anything outside the unreserved set, so a city with a space
// ("New York") or an accent does not produce a malformed request line.
static String url_encode(const char *s) {
  String out;
  for (const char *p = s; *p; ++p) {
    unsigned char c = (unsigned char)*p;
    if (isalnum(c) || c == '-' || c == '_' || c == '.' || c == '~') {
      out += (char)c;
    } else {
      char buf[4];
      snprintf(buf, sizeof buf, "%%%02X", c);
      out += buf;
    }
  }
  return out;
}

// One plain-HTTP GET. Shared by the geocode and forecast calls so there is a
// single place that speaks HTTP — and so the twin's cleartext egress path is
// exercised identically by both.
static bool http_get(const char *host, const String &path, String &out) {
  WiFiClient c;
  if (!c.connect(host, 80)) {
    Serial.print("HTTP connect() failed: ");
    Serial.println(host);
    return false;
  }
  c.print(String("GET ") + path + " HTTP/1.1\r\nHost: " + host +
          "\r\nConnection: close\r\n\r\n");

  String resp;
  unsigned long dl = millis() + 10000;
  while (millis() < dl && (c.connected() || c.available())) {
    while (c.available()) resp += (char)c.read();
    delay(10);
  }
  c.stop();
  int s = resp.indexOf("\r\n\r\n");
  out = (s >= 0) ? resp.substring(s + 4) : resp;
  Serial.print("HTTP BODY: ");
  Serial.println(out);
  return out.length() > 0;
}

// CITY -> coordinates, via Open-Meteo's geocoder (also free, also no key).
// Resolved once per boot; a city's location does not move between refreshes.
static bool resolve_city() {
  if (!isnan(g_lat) && !isnan(g_lon)) return true;

  String body;
  String path = String("/v1/search?name=") + url_encode(CITY) +
                "&count=1&language=en&format=json";
  if (!http_get(GEO_HOST, path, body)) return false;

  // Anchor inside results[0]; a miss returns {"generationtime_ms":...} with no
  // results at all, which must read as "not found" rather than as 0,0.
  int r = body.indexOf("\"results\":[");
  if (r < 0) {
    Serial.println("city not found");
    return false;
  }
  float lat = json_float(body, "latitude", r);
  float lon = json_float(body, "longitude", r);
  if (isnan(lat) || isnan(lon)) {
    Serial.println("geocode parse failed");
    return false;
  }
  g_lat = lat;
  g_lon = lon;
  Serial.printf("GEOCODED %s -> %.4f, %.4f\n", CITY, g_lat, g_lon);
  return true;
}

static bool http_get_forecast(String &out) {
  if (!resolve_city()) return false;
  String path = String("/v1/forecast?latitude=") + String(g_lat, 4) +
                "&longitude=" + String(g_lon, 4) +
                "&current=temperature_2m,relative_humidity_2m,weather_code"
                "&daily=temperature_2m_max,temperature_2m_min"
                "&timezone=auto&forecast_days=1";
  if (!http_get(API_HOST, path, out)) return false;
  return out.indexOf("\"current\":") >= 0;
}

static bool connect_wifi() {
  if (WiFi.status() == WL_CONNECTED) return true;
  WiFi.mode(WIFI_STA);
  if (WIFI_PASS && WIFI_PASS[0]) {
    WiFi.begin(WIFI_SSID, WIFI_PASS);
  } else {
    WiFi.begin(WIFI_SSID);
  }
  Serial.print("connecting to ");
  Serial.println(WIFI_SSID);
  unsigned long dl = millis() + 30000;
  while (WiFi.status() != WL_CONNECTED && millis() < dl) delay(200);
  if (WiFi.status() != WL_CONNECTED) {
    Serial.println("WiFi connect timeout");
    return false;
  }
  Serial.print("STA CONNECTED, GOT IP ");
  Serial.println(WiFi.localIP());
  return true;
}

static void refresh() {
  if (!connect_wifi()) {
    draw_offline("NO WIFI - CHECK SSID");
    return;
  }

  // Resolve first so a typo'd CITY says so, instead of blaming the forecast.
  if (!resolve_city()) {
    draw_offline("CITY NOT FOUND");
    return;
  }

  String body;
  if (!http_get_forecast(body)) {
    Serial.println("forecast fetch failed");
    draw_offline("FORECAST FETCH FAILED");
    return;
  }

  int cur = section_start(body, "current");
  int day = section_start(body, "daily");

  Weather w;
  w.temp = json_float(body, "temperature_2m", cur);
  w.humidity = json_int(body, "relative_humidity_2m", cur);
  w.code = json_int(body, "weather_code", cur);
  w.hi = json_float(body, "temperature_2m_max", day);
  w.lo = json_float(body, "temperature_2m_min", day);
  json_date(body, cur, &w.year, &w.month, &w.day);

  Serial.printf("PARSED temp=%.1f rh=%d code=%d hi=%.1f lo=%.1f date=%04d-%02d-%02d (%s)\n",
                w.temp, w.humidity, w.code, w.hi, w.lo, w.year, w.month, w.day,
                w.year ? weekday_name(w.year, w.month, w.day) : "?");

  if (isnan(w.temp)) {
    draw_offline("BAD FORECAST DATA");
    return;
  }
  draw_weather(w);
}

void setup() {
  Serial.begin(115200);
  delay(100);
  Serial.println("E-Paper Weather Station boot (GxEPD2_290_C90c / ssd1680_tricolor_290 twin)");

  // ESP32-C3: bind SPI to the diagram pins before GxEPD2 init.
  SPI.begin(PIN_SCK, /*MISO*/ -1, PIN_MOSI, PIN_CS);

  Serial.printf("pins: SCK=%d MOSI=%d CS=%d DC=%d RST=%d BUSY=%d\n",
                PIN_SCK, PIN_MOSI, PIN_CS, PIN_DC, PIN_RST, PIN_BUSY);
  pinMode(PIN_BUSY, INPUT);
  Serial.print("BUSY initial: ");
  Serial.println(digitalRead(PIN_BUSY) ? "HIGH" : "LOW");

  display.init(115200);
  display.setRotation(1);      // landscape 296x128 — same as working e-reader
  display.setTextWrap(false);  // never strand an overflowing character at x=0

  draw_boot("CONNECTING WIFI");
  refresh();
}

void loop() {
  static unsigned long last = millis();
  const unsigned long period = REFRESH_MINUTES * 60UL * 1000UL;
  // Unsigned subtraction, so this survives the millis() rollover.
  if (millis() - last >= period) {
    last = millis();
    refresh();
  }
  delay(1000);
}
