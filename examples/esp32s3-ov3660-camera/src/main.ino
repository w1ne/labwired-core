// OV3660 camera -> 1.9" ST7789 170x320 live preview, ESP32-S3 N16R8 camera board.
// Camera pins: the board's DVP bus (left header 4-13, 15-18).
// Display pins: right header, clear of the camera bus.
#include "esp_camera.h"
#include <SPI.h>

#define CAM_PIN_PWDN -1
#define CAM_PIN_RESET -1
#define CAM_PIN_XCLK 15
#define CAM_PIN_SIOD 4
#define CAM_PIN_SIOC 5
#define CAM_PIN_D7 16
#define CAM_PIN_D6 17
#define CAM_PIN_D5 18
#define CAM_PIN_D4 12
#define CAM_PIN_D3 10
#define CAM_PIN_D2 8
#define CAM_PIN_D1 9
#define CAM_PIN_D0 11
#define CAM_PIN_VSYNC 6
#define CAM_PIN_HREF 7
#define CAM_PIN_PCLK 13

static const int LCD_SCK = 39;
static const int LCD_MOSI = 40;
static const int LCD_CS = 41;
static const int LCD_DC = 42;
static const int LCD_RES = 47;
static const int LCD_BLK = 21;

static const int LCD_W = 170;
static const int LCD_H = 320;
static const int COL_OFFSET = 35;  // 170-wide glass inside the 240-wide ST7789 RAM

SPIClass lcdSpi(FSPI);
static int frames = 0;

static void lcdCmd(uint8_t c, const uint8_t *d = nullptr, int n = 0) {
  digitalWrite(LCD_DC, LOW);
  digitalWrite(LCD_CS, LOW);
  lcdSpi.transfer(c);
  digitalWrite(LCD_DC, HIGH);
  for (int i = 0; i < n; i++) lcdSpi.transfer(d[i]);
  digitalWrite(LCD_CS, HIGH);
}

static void lcdInit() {
  pinMode(LCD_DC, OUTPUT);
  pinMode(LCD_RES, OUTPUT);
  pinMode(LCD_CS, OUTPUT);
  pinMode(LCD_BLK, OUTPUT);
  digitalWrite(LCD_CS, HIGH);
  digitalWrite(LCD_BLK, HIGH);
  lcdSpi.begin(LCD_SCK, -1, LCD_MOSI, LCD_CS);
  lcdSpi.beginTransaction(SPISettings(40000000, MSBFIRST, SPI_MODE0));
  digitalWrite(LCD_RES, LOW);
  delay(10);
  digitalWrite(LCD_RES, HIGH);
  delay(120);
  lcdCmd(0x11);                 // SLPOUT
  delay(120);
  uint8_t colmod = 0x55;        // RGB565
  lcdCmd(0x3A, &colmod, 1);
  uint8_t madctl = 0x00;        // portrait 170x320
  lcdCmd(0x36, &madctl, 1);
  lcdCmd(0x21);                 // INVON
  lcdCmd(0x13);                 // NORON
  lcdCmd(0x29);                 // DISPON
  delay(20);
}

// Show a big-endian RGB565 frame (as the camera delivers it) centred on the panel.
static void lcdShowFrame(const uint8_t *buf, int w, int h) {
  int x0 = (LCD_W - w) / 2;
  int y0 = (LCD_H - h) / 2;
  uint8_t ca[4] = {0, (uint8_t)(x0 + COL_OFFSET), 0, (uint8_t)(x0 + w - 1 + COL_OFFSET)};
  lcdCmd(0x2A, ca, 4);
  uint8_t ra[4] = {0, (uint8_t)y0, 0, (uint8_t)(y0 + h - 1)};
  lcdCmd(0x2B, ra, 4);
  digitalWrite(LCD_DC, LOW);
  digitalWrite(LCD_CS, LOW);
  lcdSpi.transfer(0x2C);        // RAMWR
  digitalWrite(LCD_DC, HIGH);
  lcdSpi.writeBytes(buf, w * h * 2);
  digitalWrite(LCD_CS, HIGH);
}

static bool cameraInit() {
  camera_config_t cfg = {};
  cfg.ledc_channel = LEDC_CHANNEL_0;
  cfg.ledc_timer = LEDC_TIMER_0;
  cfg.pin_d0 = CAM_PIN_D0;
  cfg.pin_d1 = CAM_PIN_D1;
  cfg.pin_d2 = CAM_PIN_D2;
  cfg.pin_d3 = CAM_PIN_D3;
  cfg.pin_d4 = CAM_PIN_D4;
  cfg.pin_d5 = CAM_PIN_D5;
  cfg.pin_d6 = CAM_PIN_D6;
  cfg.pin_d7 = CAM_PIN_D7;
  cfg.pin_xclk = CAM_PIN_XCLK;
  cfg.pin_pclk = CAM_PIN_PCLK;
  cfg.pin_vsync = CAM_PIN_VSYNC;
  cfg.pin_href = CAM_PIN_HREF;
  cfg.pin_sccb_sda = CAM_PIN_SIOD;
  cfg.pin_sccb_scl = CAM_PIN_SIOC;
  cfg.pin_pwdn = CAM_PIN_PWDN;
  cfg.pin_reset = CAM_PIN_RESET;
  cfg.xclk_freq_hz = 20000000;
  cfg.pixel_format = PIXFORMAT_RGB565;
  cfg.frame_size = FRAMESIZE_QQVGA;   // 160x120
  cfg.jpeg_quality = 12;
  cfg.fb_count = 2;
  cfg.fb_location = CAMERA_FB_IN_PSRAM;
  cfg.grab_mode = CAMERA_GRAB_LATEST;
  return esp_camera_init(&cfg) == ESP_OK;
}

void setup() {
  Serial.begin(115200);
  lcdInit();
  if (!cameraInit()) {
    Serial.println("camera init FAILED");
    return;
  }
  sensor_t *s = esp_camera_sensor_get();
  Serial.printf("camera PID=0x%x\n", s->id.PID);
}

void loop() {
  camera_fb_t *fb = esp_camera_fb_get();
  if (!fb) {
    Serial.println("frame grab failed");
    delay(200);
    return;
  }
  lcdShowFrame(fb->buf, fb->width, fb->height);
  esp_camera_fb_return(fb);
  frames++;
  Serial.printf("frame %d shown\n", frames);
  if (frames == 3) {
    Serial.println("CAMERA_PREVIEW_OK");
  }
}
