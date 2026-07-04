/* ESP32-C3 + SSD1306 OLED demo — the curated "OLED lab" flash image.
 *
 * A real ESP-IDF application (bootloader + partition table + app) that the
 * LabWired browser twin boots FAITHFULLY through the genuine ESP32-C3 mask ROM
 * and 2nd-stage bootloader (rom-boot), then paints a recognisable frame on the
 * SSD1306 panel over the behavioral C3 I²C0 command-list controller.
 *
 * The circuit is the user's real lab (share k72DJPSUG0JV): ESP32-C3 Super Mini
 * + SSD1306 128×64 OLED on I²C, SDA=GPIO4 / SCL=GPIO5 / 3V3 / GND, panel 0x3C.
 *
 * The OLED is driven by the same register-level command-list driver the leo
 * air-quality lab uses (ssd1306_c3.c): every byte is a genuine I²C transaction
 * the simulated controller executes against the attached panel model — no
 * thunks, no faked framebuffer. The picture is what the sim's SSD1306 GDDRAM
 * actually holds after the driver's writes.
 */
#include <stdint.h>
#include <string.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_log.h"

#include "ssd1306_c3.h"
#include "font5x7.h"

static const char *TAG = "oled-lab";

static uint8_t g_fb[SSD1306_FB_SIZE];

/* Draw an upper-cased 5×7 string into the page-major framebuffer at (x, page).
 * Same renderer as the leo lab. */
static void gfx_text(uint8_t *fb, int x, int page, const char *s) {
    while (*s && x + FONT_WIDTH <= SSD1306_WIDTH) {
        char c = *s++;
        if (c >= 'a' && c <= 'z') {
            c = (char)(c - 32);
        }
        if (c < FONT_FIRST || c > FONT_LAST) {
            c = ' ';
        }
        const uint8_t *g = FONT5X7[c - FONT_FIRST];
        for (int i = 0; i < FONT_WIDTH; i++) {
            fb[page * SSD1306_WIDTH + x + i] = g[i];
        }
        x += FONT_WIDTH;
        if (x < SSD1306_WIDTH) {
            fb[page * SSD1306_WIDTH + x] = 0x00; /* 1px inter-char gap */
        }
        x += 1;
    }
}

/* Set a single pixel (x in 0..127, y in 0..63) in the page-major buffer. */
static void gfx_pixel(uint8_t *fb, int x, int y) {
    if (x < 0 || x >= SSD1306_WIDTH || y < 0 || y >= SSD1306_PAGES * 8) {
        return;
    }
    fb[(y >> 3) * SSD1306_WIDTH + x] |= (uint8_t)(1u << (y & 7));
}

/* A 1px rectangle border, inclusive of both corners. */
static void gfx_frame(uint8_t *fb, int x0, int y0, int x1, int y1) {
    for (int x = x0; x <= x1; x++) {
        gfx_pixel(fb, x, y0);
        gfx_pixel(fb, x, y1);
    }
    for (int y = y0; y <= y1; y++) {
        gfx_pixel(fb, x0, y);
        gfx_pixel(fb, x1, y);
    }
}

/* Compose the screen: framed panel with the "LabWired" wordmark, a subtitle,
 * and a filled progress bar so the frame is unmistakably non-blank. */
static void render_screen(uint8_t *fb) {
    memset(fb, 0, SSD1306_FB_SIZE);

    /* Outer frame around the whole 128×64 panel. */
    gfx_frame(fb, 0, 0, 127, 63);

    /* Wordmark + subtitle, centred-ish. */
    gfx_text(fb, 22, 2, "LABWIRED");
    gfx_text(fb, 16, 4, "OLED LAB C3");

    /* A filled bar along page 6 to guarantee a large lit-pixel count. */
    for (int x = 8; x <= 119; x++) {
        fb[6 * SSD1306_WIDTH + x] = 0x7E; /* rows 1..6 of the page lit */
    }
}

void app_main(void) {
    ESP_LOGI(TAG, "oled-lab app_main entered");
    ssd1306_init();

    render_screen(g_fb);
    ssd1306_flush(g_fb);
    ESP_LOGI(TAG, "OLED painted: LabWired");

    /* Keep refreshing so a late reader still sees the frame. */
    while (1) {
        ssd1306_flush(g_fb);
        vTaskDelay(pdMS_TO_TICKS(500));
    }
}
