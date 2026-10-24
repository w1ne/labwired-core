// Espressif's take_picture (esp32-camera examples/camera_example) in JPEG mode:
// the example's camera_config with pixel_format = PIXFORMAT_JPEG, everything
// else unchanged apart from this ESP32-S3 N16R8 camera board's pins.
// The first frame is hex-dumped between JPEG_BEGIN / JPEG_END so the bytes the
// firmware received can be decoded on the host.
#include "esp_camera.h"

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

static camera_config_t camera_config = {
    .pin_pwdn = CAM_PIN_PWDN,
    .pin_reset = CAM_PIN_RESET,
    .pin_xclk = CAM_PIN_XCLK,
    .pin_sccb_sda = CAM_PIN_SIOD,
    .pin_sccb_scl = CAM_PIN_SIOC,
    .pin_d7 = CAM_PIN_D7,
    .pin_d6 = CAM_PIN_D6,
    .pin_d5 = CAM_PIN_D5,
    .pin_d4 = CAM_PIN_D4,
    .pin_d3 = CAM_PIN_D3,
    .pin_d2 = CAM_PIN_D2,
    .pin_d1 = CAM_PIN_D1,
    .pin_d0 = CAM_PIN_D0,
    .pin_vsync = CAM_PIN_VSYNC,
    .pin_href = CAM_PIN_HREF,
    .pin_pclk = CAM_PIN_PCLK,
    .xclk_freq_hz = 20000000,
    .ledc_timer = LEDC_TIMER_0,
    .ledc_channel = LEDC_CHANNEL_0,
    .pixel_format = PIXFORMAT_JPEG,
    .frame_size = FRAMESIZE_QVGA,
    .jpeg_quality = 12,
    .fb_count = 1,
    .fb_location = CAMERA_FB_IN_PSRAM,
    .grab_mode = CAMERA_GRAB_WHEN_EMPTY,
};

static esp_err_t init_camera(void) {
  esp_err_t err = esp_camera_init(&camera_config);
  if (err != ESP_OK) {
    Serial.printf("Camera Init Failed 0x%x\n", err);
    return err;
  }
  return ESP_OK;
}

static int shots = 0;

void setup() {
  Serial.begin(115200);
  if (ESP_OK != init_camera()) {
    return;
  }
  sensor_t *s = esp_camera_sensor_get();
  Serial.printf("sensor PID=0x%x\n", s->id.PID);
}

void loop() {
  Serial.println("Taking picture...");
  camera_fb_t *pic = esp_camera_fb_get();
  if (!pic) {
    Serial.println("Picture failed");
    delay(100);
    return;
  }
  const uint8_t *b = pic->buf;
  size_t n = pic->len;
  Serial.printf("Picture taken! Its size was: %u bytes (%ux%u fmt=%d) head=%02x%02x%02x tail=%02x%02x\n",
                (unsigned)n, pic->width, pic->height, pic->format,
                b[0], b[1], b[2], b[n - 2], b[n - 1]);
  if (shots == 0) {
    Serial.println("JPEG_BEGIN");
    for (size_t i = 0; i < n; i++) {
      Serial.printf("%02x", b[i]);
      if ((i & 63) == 63) Serial.println();
    }
    Serial.println();
    Serial.println("JPEG_END");
  }
  esp_camera_fb_return(pic);
  shots++;
  if (shots == 3) {
    Serial.println("TAKE_PICTURE_JPEG_OK");
  }
  delay(100);
}
