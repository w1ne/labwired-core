// Minimal arduino-esp32 WiFi fixture for the LabWired simulated-endpoints
// WiFi model: join an AP, HTTP GET an in-sim server, print the result.
#include <WiFi.h>
#include <HTTPClient.h>

void setup() {
  Serial.begin(115200);
  WiFi.begin("labwired", "hunter2");
  while (WiFi.status() != WL_CONNECTED) {
    delay(100);
  }
  Serial.println("WIFI OK");
  Serial.println(WiFi.localIP());

  HTTPClient http;
  http.begin("http://192.168.4.1/status");
  int code = http.GET();
  Serial.printf("HTTP %d\n", code);
  if (code > 0) {
    Serial.println(http.getString());
  }
  http.end();
}

void loop() {}
