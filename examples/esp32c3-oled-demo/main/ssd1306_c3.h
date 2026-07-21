/* Minimal SSD1306 128×64 OLED driver for the ESP32-C3 sim.
 *
 * Drives the same simulated C3 I²C0 command-list engine as the sensor HAL. The
 * panel is the on-device "screen" that shows Leo's plain-language air verdict.
 */
#ifndef SSD1306_C3_H
#define SSD1306_C3_H

#include <stdint.h>

#define SSD1306_I2C_ADDR 0x3C
#define SSD1306_WIDTH 128
#define SSD1306_PAGES 8
#define SSD1306_FB_SIZE (SSD1306_WIDTH * SSD1306_PAGES) /* 1024 bytes, page-major */

/* Standard power-on init (display off → addressing → charge pump → display on). */
void ssd1306_init(void);

/* Push a 1024-byte page-major framebuffer to the panel (chunked to the C3's
 * 32-byte I²C FIFO). */
void ssd1306_flush(const uint8_t *fb);

#endif /* SSD1306_C3_H */
