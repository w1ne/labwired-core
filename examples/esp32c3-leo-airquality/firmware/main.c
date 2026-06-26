/* Leo air-quality sensor firmware — ESP32-C3 (RISC-V rv32imc).
 *
 * Boots on a simulated ESP32-C3, reads four air-quality sensors over the REAL
 * C3 I²C0 controller, and turns the raw measurements into a plain-language
 * verdict over UART0 — exactly Leo's product promise ("translates air data into
 * plain language"), running with no hardware in the loop.
 *
 * Three of the four drivers are the UNMODIFIED Sensirion embedded-i2c vendor
 * libraries running ON-TARGET (riscv32):
 *   - SCD41  CO₂ + temperature + humidity   (third_party/embedded-i2c-scd4x)
 *   - SGP41  VOC raw → VOC Index             (third_party/embedded-i2c-sgp41 +
 *                                             gas-index-algorithm)
 *   - SPS30  particulate matter (PM2.5 …)    (third_party/embedded-i2c-sps30)
 * The VEML7700 ambient-light driver is a small register-level driver (Vishay
 * ships no bare-metal C library). All four talk to the sensor device models
 * through genuine I²C transactions executed by the simulated C3 controller.
 *
 * The headline story is a room filling up: CO₂ climbs from fresh toward stuffy
 * and the verdict flips from "air quality is good" to "CO₂ climbing, crack a
 * window" — live, deterministic, reproducible.
 */
#include <stdbool.h>
#include <stdint.h>

#include "c3_uart.h"
#include "scd4x_i2c.h"
#include "sensirion_gas_index_algorithm.h"
#include "sensirion_i2c_hal.h"
#include "sgp41_i2c.h"
#include "sps30_i2c.h"
#include "ssd1306_c3.h"
#include "veml7700_c3.h"
#include "font5x7.h"

#define SCD41_ADDR 0x62
#define SPS30_ADDR 0x69

/* Number of measurement cycles to run before the demo halts. The CO₂ ramp
 * (start 450 → target 1400 ppm, alpha 0.08) crosses 1000 ppm by ~cycle 11, so
 * 64 cycles comfortably show the full fresh → stuffy transition. */
#define SAMPLES 64

/* Default RH/T compensation inputs for the SGP41 raw command, in the sensor's
 * tick encoding (50 %RH, 25 °C) per the datasheet — what a real integration
 * passes before the on-board humidity reading is wired in. */
#define SGP41_DEFAULT_RH 0x8000
#define SGP41_DEFAULT_T 0x6666

/* Emit the plain-language air verdict. CO₂ is the headline; particulates add a
 * secondary clause when they matter. Thresholds are firmware policy, blind to
 * the simulator's scene. */
static void print_verdict(uint16_t co2, uint16_t pm2_5) {
    uart_puts("AIR: ");
    if (co2 >= 1400) {
        uart_puts("stale air - ventilate now, CO2 is high");
    } else if (co2 >= 1000) {
        uart_puts("getting stuffy - CO2 climbing, crack a window");
    } else if (co2 >= 800) {
        uart_puts("okay - air is fine but CO2 is creeping up");
    } else {
        uart_puts("fresh - air quality is good");
    }
    if (pm2_5 >= 35) {
        uart_puts("; particulates unhealthy");
    } else if (pm2_5 >= 12) {
        uart_puts("; some haze in the air");
    }
    uart_puts("\r\n");
}

/* ── Mold risk ───────────────────────────────────────────────────────────────
 * Mold is not a sensor reading — commercial IAQ monitors (e.g. the ThinkLite
 * Flair's "Mold Index") derive it. Mould germinates when humidity stays high in
 * a livable temperature band; the longer a room sits damp, the higher the risk.
 * We track a dwell counter of consecutive mold-favorable cycles and combine it
 * with the humidity level — all from the SCD41's T/RH, no extra hardware. */
static int g_damp_dwell = 0; /* consecutive cycles in mold-favorable conditions */

static int mold_favorable(int temp_c, int rh) {
    return temp_c >= 10 && temp_c <= 35 && rh >= 60;
}

/* 0 = low … 4 = severe. */
static int mold_index(int temp_c, int rh) {
    if (!mold_favorable(temp_c, rh)) {
        if (g_damp_dwell > 0) {
            g_damp_dwell--;
        }
        return rh >= 55 ? 1 : 0;
    }
    if (g_damp_dwell < 240) {
        g_damp_dwell++;
    }
    int idx = 1; /* damp + livable temperature */
    if (rh >= 70) {
        idx++; /* very damp */
    }
    if (g_damp_dwell >= 6) {
        idx++; /* sustained */
    }
    if (rh >= 70 && g_damp_dwell >= 12) {
        idx++; /* sustained AND very damp */
    }
    return idx > 4 ? 4 : idx;
}

static const char *mold_verdict(int idx) {
    switch (idx) {
        case 0:
            return "mold risk: low";
        case 1:
            return "mold risk: watch - humidity climbing";
        case 2:
            return "mold risk: ELEVATED - damp, mold-favorable";
        case 3:
            return "mold risk: HIGH - sustained damp";
        default:
            return "mold risk: SEVERE - mold likely to grow";
    }
}

/* ── On-device OLED screen ───────────────────────────────────────────────── */

static uint8_t g_fb[SSD1306_FB_SIZE];

static int put_str(char *d, int p, const char *s) {
    while (*s) {
        d[p++] = *s++;
    }
    return p;
}

static int put_uint(char *d, int p, uint32_t v) {
    char t[12];
    int n = 0;
    if (v == 0) {
        t[n++] = '0';
    }
    while (v) {
        t[n++] = (char)('0' + (v % 10u));
        v /= 10u;
    }
    while (n) {
        d[p++] = t[--n];
    }
    return p;
}

/* Draw an upper-cased string into the page-major framebuffer at (x, page). */
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

/* Short OLED headline. Mold risk takes the headline once it is elevated — that
 * is the metric Leo's users care about most; otherwise CO₂ leads. */
static const char *oled_headline(uint16_t co2, int mold) {
    if (mold >= 3) {
        return ">MOLD RISK HIGH";
    } else if (mold >= 2) {
        return ">MOLD RISK UP";
    }
    if (co2 >= 1400) {
        return ">VENTILATE NOW";
    } else if (co2 >= 1000) {
        return ">CRACK A WINDOW";
    } else if (co2 >= 800) {
        return ">CO2 RISING";
    }
    return ">AIR IS GOOD";
}

static void render_screen(uint16_t co2, int temp_c, int rh, uint16_t pm2_5,
                          int32_t voc, uint32_t lux, int mold) {
    for (int i = 0; i < SSD1306_FB_SIZE; i++) {
        g_fb[i] = 0;
    }
    char line[24];
    int p;

    gfx_text(g_fb, 0, 0, "LEO AIR QUALITY");

    p = put_str(line, 0, "CO2   ");
    p = put_uint(line, p, co2);
    p = put_str(line, p, " PPM");
    line[p] = 0;
    gfx_text(g_fb, 0, 1, line);

    p = put_str(line, 0, "PM2.5  ");
    p = put_uint(line, p, pm2_5);
    p = put_str(line, p, " UG");
    line[p] = 0;
    gfx_text(g_fb, 0, 2, line);

    p = put_str(line, 0, "VOC    ");
    p = put_uint(line, p, (uint32_t)(voc < 0 ? 0 : voc));
    line[p] = 0;
    gfx_text(g_fb, 0, 3, line);

    p = put_str(line, 0, "LIGHT ");
    p = put_uint(line, p, lux);
    p = put_str(line, p, " LX");
    line[p] = 0;
    gfx_text(g_fb, 0, 4, line);

    p = put_str(line, 0, "TEMP ");
    p = put_uint(line, p, (uint32_t)temp_c);
    p = put_str(line, p, "C RH ");
    p = put_uint(line, p, (uint32_t)rh);
    p = put_str(line, p, "%");
    line[p] = 0;
    gfx_text(g_fb, 0, 5, line);

    p = put_str(line, 0, "MOLD  ");
    p = put_str(line, p, mold >= 3 ? "HIGH" : mold >= 2 ? "ELEVATED"
                                            : mold >= 1 ? "WATCH"
                                                        : "LOW");
    line[p] = 0;
    gfx_text(g_fb, 0, 6, line);

    gfx_text(g_fb, 0, 7, oled_headline(co2, mold));
}

/* Dump the framebuffer as ASCII art over UART so the rendered screen is
 * verifiable in headless runs (and legible in the log). */
static void oled_dump_ascii(void) {
    uart_puts("OLED-FB-BEGIN\r\n");
    for (int y = 0; y < 64; y++) {
        int page = y >> 3;
        int bit = y & 7;
        for (int x = 0; x < SSD1306_WIDTH; x++) {
            uint8_t on = (uint8_t)((g_fb[page * SSD1306_WIDTH + x] >> bit) & 1u);
            uart_putc(on ? '#' : ' ');
        }
        uart_puts("\r\n");
    }
    uart_puts("OLED-FB-END\r\n");
}

int main(void) {
    sensirion_i2c_hal_init();
    uart_puts("LEO BOOT\r\n");
    uart_puts("Leo air-quality sensor: ESP32-C3 + SCD41/SGP41/SPS30 + VEML7700\r\n");

    /* ── SCD41: CO₂ + T + RH ──────────────────────────────────────────────── */
    scd4x_init(SCD41_ADDR);
    scd4x_wake_up();
    scd4x_stop_periodic_measurement();
    scd4x_reinit();
    scd4x_start_periodic_measurement();
    uart_puts("SCD41 READY\r\n");

    /* ── SGP41: VOC/NOx raw + Sensirion Gas Index Algorithm ───────────────── */
    uint16_t sraw_voc_cond = 0;
    sgp41_execute_conditioning(SGP41_DEFAULT_RH, SGP41_DEFAULT_T, &sraw_voc_cond);
    uart_puts("SGP41 READY\r\n");

    /* ── SPS30: particulate matter (integer/uint16 output, 30-byte frame) ──── */
    sps30_init(SPS30_ADDR);
    sps30_wake_up();
    sps30_start_measurement(
        (sps30_output_format)SPS30_OUTPUT_FORMAT_OUTPUT_FORMAT_UINT16);
    uart_puts("SPS30 READY\r\n");

    /* ── VEML7700: ambient light ─────────────────────────────────────────── */
    veml7700_init();
    uart_puts("VEML7700 READY\r\n");

    ssd1306_init();
    uart_puts("OLED READY\r\n");

    GasIndexAlgorithmParams voc_params;
    GasIndexAlgorithm_init(&voc_params, GasIndexAlgorithm_ALGORITHM_TYPE_VOC);

    for (int cycle = 0; cycle < SAMPLES; cycle++) {
        /* SCD41 — CO₂ ppm, temperature (m°C), humidity (m%RH). */
        bool ready = false;
        uint16_t co2 = 0;
        int32_t temp_m_deg_c = 0;
        int32_t rh_m_pct = 0;
        scd4x_get_data_ready_status(&ready);
        scd4x_read_measurement(&co2, &temp_m_deg_c, &rh_m_pct);

        /* SGP41 — raw VOC ticks → VOC Index via the real gas-index algorithm. */
        uint16_t sraw_voc = 0;
        uint16_t sraw_nox = 0;
        sgp41_measure_raw_signals(SGP41_DEFAULT_RH, SGP41_DEFAULT_T, &sraw_voc,
                                  &sraw_nox);
        int32_t voc_index = 0;
        GasIndexAlgorithm_process(&voc_params, (int32_t)sraw_voc, &voc_index);

        /* SPS30 — particulate mass/number concentrations (integer mode). */
        uint16_t mc_1p0 = 0, mc_2p5 = 0, mc_4p0 = 0, mc_10p0 = 0;
        uint16_t nc_0p5 = 0, nc_1p0 = 0, nc_2p5 = 0, nc_4p0 = 0, nc_10p0 = 0;
        uint16_t typ_size = 0;
        uint16_t pm_flag = 0;
        sps30_read_data_ready_flag(&pm_flag);
        sps30_read_measurement_values_uint16(&mc_1p0, &mc_2p5, &mc_4p0, &mc_10p0,
                                             &nc_0p5, &nc_1p0, &nc_2p5, &nc_4p0,
                                             &nc_10p0, &typ_size);

        /* VEML7700 — ambient light in lux. */
        uint16_t als_counts = 0;
        veml7700_read_als(&als_counts);
        uint32_t lux = veml7700_counts_to_lux(als_counts);

        /* Per-cycle measurement line. */
        uart_puts("t=");
        uart_puti(cycle);
        uart_puts(" CO2=");
        uart_puti((int32_t)co2);
        uart_puts("ppm T=");
        uart_putfix2(temp_m_deg_c / 10); /* m°C → value×100 */
        uart_puts("C RH=");
        uart_puti(rh_m_pct / 1000); /* m%RH → % */
        uart_puts("% PM2.5=");
        uart_puti((int32_t)mc_2p5);
        uart_puts("ug VOC=");
        uart_puti(voc_index);
        uart_puts(" LUX=");
        uart_puti((int32_t)lux);
        uart_puts("\r\n");

        print_verdict(co2, mc_2p5);

        /* Mold risk from the SCD41's temperature + humidity — the "Mold Index"
         * a commercial monitor reports, derived rather than directly sensed. */
        int temp_c = (int)(temp_m_deg_c / 1000);
        int rh_pct = (int)(rh_m_pct / 1000);
        int mold = mold_index(temp_c, rh_pct);
        uart_puts("MOLD: ");
        uart_puts(mold_verdict(mold));
        uart_puts("\r\n");

        /* Render the same readings + verdict to the on-device OLED. */
        render_screen(co2, temp_c, rh_pct, mc_2p5, voc_index, lux, mold);
        ssd1306_flush(g_fb);
        if (cycle == 0) {
            uart_puts("OLED render done\r\n");
        }
    }

    uart_puts("LEO DONE\r\n");
    /* Echo the final OLED frame as ASCII art for headless verification. */
    oled_dump_ascii();
    for (;;) {
    }
}
