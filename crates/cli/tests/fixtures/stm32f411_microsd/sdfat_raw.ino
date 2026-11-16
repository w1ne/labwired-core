// Built by the hosted LabWired compile for stm32f411ceu6-blackpill with
// lib_deps greiman/SdFat@2.2.3, then stripped. SdFat's raw card driver runs
// the full SPI-mode init (CMD0, CMD8, ACMD41, CMD58), reads the CSD (CMD9),
// writes and reads one block (CMD24/CMD17) and three blocks (CMD25/CMD18).
#include <SPI.h>
#include <SdFat.h>

SdSpiCard card;
uint8_t buf[512];

static void fill(uint32_t lba) {
  for (int i = 0; i < 512; i++) buf[i] = (uint8_t)(lba * 7 + i);
}
static bool check(uint32_t lba) {
  for (int i = 0; i < 512; i++) if (buf[i] != (uint8_t)(lba * 7 + i)) return false;
  return true;
}

void setup() {
  Serial.begin(115200);
  SPI.setSCLK(PA5); SPI.setMISO(PA6); SPI.setMOSI(PA7);
  if (!card.begin(SdSpiConfig(PA4, DEDICATED_SPI, SD_SCK_MHZ(4)))) {
    Serial.print("INIT FAIL code=0x");
    Serial.print(card.errorCode(), HEX);
    Serial.print(" data=0x");
    Serial.println(card.errorData(), HEX);
    return;
  }
  Serial.print("INIT OK type=");
  Serial.print(card.type());
  Serial.print(" sectors=");
  Serial.println(card.sectorCount());

  memset(buf, 0, sizeof(buf));
  memcpy(buf, "PATHFINDER FLIGHT LOG v1", 24);
  if (!card.writeSector(100, buf)) { Serial.println("WRITE FAIL"); return; }
  memset(buf, 0, sizeof(buf));
  if (!card.readSector(100, buf) || memcmp(buf, "PATHFINDER FLIGHT LOG v1", 24) != 0) {
    Serial.println("READBACK FAIL"); return;
  }
  Serial.println("SINGLE OK");

  if (!card.writeStart(200)) { Serial.println("MW START FAIL"); return; }
  for (uint32_t i = 0; i < 3; i++) {
    fill(200 + i);
    if (!card.writeData(buf)) { Serial.println("MW DATA FAIL"); return; }
  }
  if (!card.writeStop()) { Serial.println("MW STOP FAIL"); return; }
  if (!card.readStart(200)) { Serial.println("MR START FAIL"); return; }
  for (uint32_t i = 0; i < 3; i++) {
    if (!card.readData(buf) || !check(200 + i)) { Serial.println("MR DATA FAIL"); return; }
  }
  if (!card.readStop()) { Serial.println("MR STOP FAIL"); return; }
  Serial.println("MULTI OK");
  Serial.println("SD DONE");
}
void loop() {}
