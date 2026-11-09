#include <LittleFS.h>
#include <errno.h>
#include <esp_partition.h>
static uint8_t pat(uint32_t i) { return (uint8_t)((i * 31u + 7u) ^ (i >> 8)); }
void setup() {
  Serial.begin(115200); delay(10);
  bool ok = LittleFS.begin(true);
  Serial.printf("begin=%d\n", ok);
  uint8_t buf[256];
  for (uint32_t N : {8000u}) {
    char name[16]; snprintf(name, sizeof name, "/f%u", (unsigned)N);
    File f = LittleFS.open(name, "w");
    size_t bw = 0;
    for (uint32_t off = 0; off < N; off += 256) {
      uint32_t n = (N - off) < 256 ? (N - off) : 256;
      for (uint32_t i = 0; i < n; i++) buf[i] = pat(off + i);
      bw += f.write(buf, n);
    }
    f.close();
    { const esp_partition_t *pt = esp_partition_find_first(ESP_PARTITION_TYPE_DATA, ESP_PARTITION_SUBTYPE_DATA_SPIFFS, NULL);
      for (uint32_t o = 0; o < 0x160000; o += 0x1000) { uint32_t w[4]; esp_partition_read(pt, o, w, 16);
        if (w[0] != 0xffffffffu) Serial.printf("raw %05x %08x %08x %08x %08x\n", (unsigned)o, (unsigned)w[0], (unsigned)w[1], (unsigned)w[2], (unsigned)w[3]); } }
    { const esp_partition_t *pt = esp_partition_find_first(ESP_PARTITION_TYPE_DATA, ESP_PARTITION_SUBTYPE_DATA_SPIFFS, NULL);
      for (uint32_t o : {0xBD000u, 0xBD004u, 0xBD008u, 0xBD001u}) { uint32_t w[2] = {0, 0}; esp_partition_read(pt, o, w, 8);
        Serial.printf("un %05x %08x %08x\n", (unsigned)o, (unsigned)w[0], (unsigned)w[1]); } }
    f = LittleFS.open(name, "r");
    uint32_t br = 0, bad = 0; errno = 0;
    while (f.available()) { int n = f.read(buf, 256); if (n <= 0) break;
      for (int i = 0; i < n; i++) if (buf[i] != pat(br + i)) { if (bad < 6) Serial.printf("bad@%u got=%02x want=%02x\n", (unsigned)(br + i), buf[i], pat(br + i)); bad++; } br += n; }
    int e = errno; f.close();
    Serial.printf("N=%u w=%u r=%u bad=%u errno=%d\n", (unsigned)N, (unsigned)bw, (unsigned)br, (unsigned)bad, e);
    LittleFS.remove(name);
  }
  Serial.println("LW_SZ_DONE");
}
void loop() { delay(1000); }
