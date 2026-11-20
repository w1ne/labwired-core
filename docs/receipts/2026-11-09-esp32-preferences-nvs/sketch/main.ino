// LabWired ESP32 Preferences (NVS) round-trip — stock arduino-esp32 3.x.
// 800 overwrites of one key fill the 5-page NVS partition several times
// over, so NVS must reclaim pages (sector erase) to keep going.
#include <Preferences.h>
Preferences prefs;
void setup() {
  Serial.begin(115200);
  delay(10);
  Serial.println("LW_NVS_BOOT");
  bool ok = prefs.begin("lw", false);
  uint32_t boots = prefs.getUInt("boots", 0);
  size_t w = prefs.putUInt("boots", boots + 1);
  uint32_t rb = prefs.getUInt("boots", 0);
  Serial.printf("begin=%d boots=%u put=%u readback=%u\n", ok, boots, (unsigned)w, rb);
  prefs.putString("name", "labwired");
  String name = prefs.getString("name", "");
  Serial.printf("name=%s\n", name.c_str());
  for (uint32_t i = 0; i < 800; i++) prefs.putUInt("ctr", i);
  uint32_t ctr = prefs.getUInt("ctr", 0);
  uint32_t rb2 = prefs.getUInt("boots", 0);
  String name2 = prefs.getString("name", "");
  Serial.printf("ctr=%u boots=%u name=%s free=%u\n", ctr, rb2, name2.c_str(), (unsigned)prefs.freeEntries());
  prefs.remove("name");
  bool gone = !prefs.isKey("name");
  Serial.printf("removed=%d\n", gone);
  prefs.end();
  if (ok && w == 4 && rb == boots + 1 && name == "labwired" && ctr == 799 &&
      rb2 == boots + 1 && name2 == "labwired" && gone)
    Serial.println("LW_NVS_OK");
  else
    Serial.println("LW_NVS_FAIL");
}
void loop() { delay(1000); }
