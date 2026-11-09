// LabWired ESP32 LittleFS round-trip — stock arduino-esp32 3.x.
// Formats the "spiffs" partition, writes, appends, renames and removes
// files, then writes a 24 KiB file (six 4 KiB blocks) and reads it back.
#include <LittleFS.h>

static uint8_t pat(uint32_t i) { return (uint8_t)((i * 31u + 7u) ^ (i >> 8)); }

void setup() {
  Serial.begin(115200);
  delay(10);
  Serial.println("LW_FS_BOOT");
  bool ok = LittleFS.begin(true);
  Serial.printf("begin=%d total=%u used=%u\n", ok, (unsigned)LittleFS.totalBytes(),
                (unsigned)LittleFS.usedBytes());

  File f = LittleFS.open("/hello.txt", "w");
  size_t w = f ? f.print("hello labwired\n") : 0;
  if (f) f.close();
  f = LittleFS.open("/hello.txt", "a");
  size_t a = f ? f.print("second line\n") : 0;
  if (f) f.close();
  f = LittleFS.open("/hello.txt", "r");
  String txt = f ? f.readString() : String();
  if (f) f.close();
  Serial.printf("w=%u a=%u read=[%s]\n", (unsigned)w, (unsigned)a, txt.c_str());

  bool ren = LittleFS.rename("/hello.txt", "/greet.txt");
  bool oldgone = !LittleFS.exists("/hello.txt");
  bool newhere = LittleFS.exists("/greet.txt");

  const uint32_t N = 24 * 1024;
  uint8_t buf[256];
  f = LittleFS.open("/big.bin", "w");
  size_t bw = 0;
  for (uint32_t off = 0; f && off < N; off += sizeof(buf)) {
    for (uint32_t i = 0; i < sizeof(buf); i++) buf[i] = pat(off + i);
    bw += f.write(buf, sizeof(buf));
  }
  if (f) f.close();
  f = LittleFS.open("/big.bin", "r");
  size_t sz = f ? f.size() : 0;
  uint32_t bad = 0, br = 0;
  while (f && f.available()) {
    int n = f.read(buf, sizeof(buf));
    if (n <= 0) break;
    for (int i = 0; i < n; i++) if (buf[i] != pat(br + i)) bad++;
    br += n;
  }
  if (f) f.close();
  Serial.printf("ren=%d old_gone=%d new=%d big_w=%u size=%u read=%u bad=%u\n", ren, oldgone,
                newhere, (unsigned)bw, (unsigned)sz, (unsigned)br, (unsigned)bad);

  int files = 0;
  File root = LittleFS.open("/");
  for (File e = root.openNextFile(); e; e = root.openNextFile()) {
    Serial.printf("ls %s %u\n", e.name(), (unsigned)e.size());
    files++;
  }
  bool rm = LittleFS.remove("/big.bin");
  bool rmgone = !LittleFS.exists("/big.bin");
  LittleFS.end();

  // Remount without format: the data must come back from flash.
  bool ok2 = LittleFS.begin(false);
  f = LittleFS.open("/greet.txt", "r");
  String txt2 = f ? f.readString() : String();
  if (f) f.close();
  Serial.printf("files=%d rm=%d rm_gone=%d remount=%d reread=[%s]\n", files, rm, rmgone, ok2,
                txt2.c_str());

  const char *want = "hello labwired\nsecond line\n";
  if (ok && txt == want && ren && oldgone && newhere && bw == N && sz == N && br == N &&
      bad == 0 && files == 2 && rm && rmgone && ok2 && txt2 == want)
    Serial.println("LW_FS_OK");
  else
    Serial.println("LW_FS_FAIL");
}

void loop() { delay(1000); }
