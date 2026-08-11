// LabWired Arduino matrix L0 — prove setup() + serial after core boot.
//
// Nano 33 BLE (Arduino mbed): Serial is SerialUSB and begin() can wait forever
// for a USB host the sim does not enumerate. Always print on Serial1 (UARTE1
// on D0/D1) which LabWired captures. Other boards use Serial (hardware UART).
#if defined(ARDUINO_ARDUINO_NANO33BLE) || defined(ARDUINO_NANO33BLE)
#define LW_SERIAL Serial1
#else
#define LW_SERIAL Serial
#endif

void setup() {
  LW_SERIAL.begin(115200);
  delay(1);
  LW_SERIAL.println("LW_L0_OK");
}

void loop() {
  // Idle. Marker is only required once from setup().
}
