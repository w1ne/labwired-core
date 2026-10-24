// LabWired climate node — STM32F411CEU6 "Black Pill" (WeAct) Arduino sketch.
//
// Board (packages/board-config/test/fixtures/designs/stm32f411ceu6-climate-node):
//   NTC (th1) top leg: VBAT -> NTC -> PA0 -> 10k (R3) -> GND   (ADC1 ch0)
//   LDR (ldr1) top leg: VBAT -> LDR -> PA1 -> 10k (R4) -> GND  (ADC1 ch1)
//   BME280 (u2) at 0x76 and SSD1306 (oled1, on the J3 header) at 0x3C on I2C1
//     (SCL PB6 / SDA PB7 — the STM32duino route pins, set before Wire.begin)
//   LED PWM on PA5 through R5, buzzer on PA6 through Q1.
//
// The serial contract io-smoke.yaml asserts (keep tokens stable):
//   NODE boot
//   BME280 ok|missing
//   OLED painted|missing
//   NTC iter=<n> adc=<counts> mV=<mv> temp_c=<c>
//   LDR adc=<counts> lux=<lux>
//   BME t=<c> p=<hPa> h=<rh>
//   ALARM 0|1
//
// Physics matches the engine's device models (core/configs/devices):
//   * ntc_thermistor.yaml is the TOP leg of a 10k pull-down divider:
//     V = Vref * R_pull / (R_ntc + R_pull), R(T) = R0 * exp(B (1/T - 1/T0)),
//     R0 = 10k, B = 3950, T0 = 298.15 K. So the ADC counts RISE with
//     temperature and sit at exactly Vref/2 (2047 counts) at 25 C.
//   * ldr.yaml is the same divider with R(lux) = 10k * (lux/10)^-0.7.
//   * bme280.yaml serves the factory calibration registers; the Adafruit
//     library's integer compensation recovers the stimulus value.
//   * ssd1306.yaml is the 128x64 panel (8-page GDDRAM, 0x3C default address),
//     so the Adafruit_SSD1306 constructor is (128, 64, &Wire, -1).

#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>
#include <Adafruit_SSD1306.h>

static const int PIN_NTC = PA0;
static const int PIN_LDR = PA1;
static const int PIN_LED = PA5;
static const int PIN_BUZZER = PA6;

static const float VREF_MV = 3300.0f;
static const float ADC_MAX = 4095.0f;
static const float R_PULL_OHM = 10000.0f;

static const float NTC_R0 = 10000.0f;
static const float NTC_B = 3950.0f;
static const float NTC_T0_K = 298.15f;

static const float LDR_R10_OHM = 10000.0f;
static const float LDR_GAMMA = 0.7f;

static const float OVER_TEMP_C = 45.0f;
static const uint8_t BME_ADDR = 0x76;
static const uint8_t OLED_ADDR = 0x3C;
static const int LOOP_DELAY_MS = 10;

Adafruit_BME280 bme;
Adafruit_SSD1306 oled(128, 64, &Wire, -1);

static bool bmeOk = false;
static bool oledOk = false;

static float adcToMv(uint16_t counts) {
  return (counts / ADC_MAX) * VREF_MV;
}

// NTC top-leg divider: V = Vref * R_pull / (R_ntc + R_pull), so
// R_ntc = R_pull * (Vref - V) / V, then Steinhart-Hart (beta form) with
// R0 = 10k at 25 C. A rail is not a reading. (The polarity matters: with the
// NTC on TOP the node RISES as it heats — at 25 C it sits at Vref/2.)
static float ntcTempC(uint16_t counts) {
  float mv = adcToMv(counts);
  if (mv <= 0.5f || mv >= VREF_MV - 0.5f) return NAN;
  float rNtc = R_PULL_OHM * (VREF_MV - mv) / mv;
  float invT = 1.0f / NTC_T0_K + (1.0f / NTC_B) * logf(rNtc / NTC_R0);
  return 1.0f / invT - 273.15f;
}

// LDR top-leg divider, inverted from the model's power law:
// R_ldr = R_pull * (Vref - V) / V, lux = 10 * (R_ldr / 10k)^(-1/gamma).
// The dark rail is lux 0 (R -> +inf); the bright rail is unreachable.
static float ldrLux(uint16_t counts) {
  float mv = adcToMv(counts);
  if (mv <= 0.5f) return 0.0f;
  if (mv >= VREF_MV - 0.5f) return NAN;
  float rLdr = R_PULL_OHM * (VREF_MV - mv) / mv;
  return 10.0f * powf(rLdr / LDR_R10_OHM, -1.0f / LDR_GAMMA);
}

void setup() {
  Serial.begin(SERIAL_BAUD);
  delay(10);
  Serial.println(F("NODE boot"));

  analogReadResolution(12);
  pinMode(PIN_LED, OUTPUT);
  pinMode(PIN_BUZZER, OUTPUT);
  analogWrite(PIN_LED, 0);
  digitalWrite(PIN_BUZZER, LOW);

  Wire.setSDA(PB7);
  Wire.setSCL(PB6);
  Wire.begin();

  bmeOk = bme.begin(BME_ADDR, &Wire);
  Serial.print(F("BME280 "));
  Serial.println(bmeOk ? F("ok") : F("missing"));

  // The whole body below runs whether or not a peripheral answered: a missing
  // device prints its token and the node keeps reporting.
  oledOk = oled.begin(SSD1306_SWITCHCAPVCC, OLED_ADDR);
  if (!oledOk) {
    Serial.println(F("OLED missing"));
  } else {
    oled.clearDisplay();
    oled.setTextColor(SSD1306_WHITE);
    oled.setTextSize(1);
    // A solid header band: the smoke's display_region assertion reads THIS
    // band, so keep it fully inked at the end of every iteration.
    oled.fillRect(0, 0, 128, 8, SSD1306_WHITE);
    oled.setCursor(0, 12);
    oled.print(F("climate node"));
    oled.display();
    Serial.println(F("OLED painted"));
  }
}

static void paintScreen(float tNtc) {
  if (!oledOk) return;
  oled.fillRect(0, 0, 128, 8, SSD1306_WHITE);
  oled.fillRect(0, 12, 128, 8, SSD1306_BLACK);
  oled.setCursor(0, 12);
  oled.print(tNtc, 1);
  oled.print(F(" C"));
  oled.display();
}

void loop() {
  static uint32_t iter = 0;

  uint16_t adcNtc = analogRead(PIN_NTC);
  uint16_t adcLdr = analogRead(PIN_LDR);
  float tNtc = ntcTempC(adcNtc);
  float lux = ldrLux(adcLdr);

  Serial.print(F("NTC iter="));
  Serial.print(iter);
  Serial.print(F(" adc="));
  Serial.print(adcNtc);
  Serial.print(F(" mV="));
  Serial.print(adcToMv(adcNtc), 1);
  Serial.print(F(" temp_c="));
  Serial.println(tNtc, 2);

  Serial.print(F("LDR adc="));
  Serial.print(adcLdr);
  Serial.print(F(" lux="));
  Serial.println(lux, 1);

  if (bmeOk) {
    Serial.print(F("BME t="));
    Serial.print(bme.readTemperature(), 2);
    Serial.print(F(" p="));
    Serial.print(bme.readPressure() / 100.0f, 1);
    Serial.print(F(" h="));
    Serial.println(bme.readHumidity(), 1);
  }

  int pwm = isnan(tNtc) ? 0 : (int)constrain((tNtc - 20.0f) * 12.75f, 0.0f, 255.0f);
  analogWrite(PIN_LED, pwm);

  bool alarm = !isnan(tNtc) && tNtc >= OVER_TEMP_C;
  digitalWrite(PIN_BUZZER, alarm ? HIGH : LOW);
  Serial.print(F("ALARM "));
  Serial.println(alarm ? 1 : 0);

  paintScreen(tNtc);

  iter++;
  delay(LOOP_DELAY_MS);
}
