/*
 * Snake on an nRF54LM20A driving an RM67162 AMOLED over SPIM22.
 *
 * What this firmware is for: it is the end-to-end proof that the nRF54LM20A
 * chip profile, the nRF54L SPIM model and the RM67162 panel model work
 * TOGETHER on a booted machine, reached through from_config. Each of those has
 * unit tests; none of them proves the others.
 *
 * Exercises, in order:
 *   1. reset -> vector table at RRAM 0x0 -> .data copy / .bss zero
 *   2. UARTE20 EasyDMA TX at 115200 -> banner on the DK console
 *   3. GPIO P1 outputs (LEDs) and P1/P0 inputs with PIN_CNF pull-ups
 *   4. SPIM22 EasyDMA -> RM67162 init -> DISPON at full brightness
 *   5. a game loop that paints cells and reads four buttons
 *
 * Everything is polled. No interrupts and no clock setup: the part comes out
 * of reset on the internal oscillator.
 *
 * ── The two things worth reading the code for ────────────────────────────
 *
 * HARDWARE D/C. Every byte to the panel goes out as ONE EasyDMA transfer whose
 * first byte is the command and whose remainder is data, with DCXCNT = 1. The
 * controller holds the panel's D/C line low for that first byte and high for
 * the rest. There is no D/C GPIO anywhere in this firmware and no pin write
 * between the command and its data -- that is what PSEL.DCX buys, and it is
 * why a pixel burst is a single bus transaction rather than a toggle sandwich.
 *
 * BRIGHTNESS. WRDISBV (0x51) is written explicitly. On a backlit TFT this
 * command does not exist and brightness is a separate backlight pin; on an
 * emissive panel it is the difference between a picture and a black screen,
 * because its reset value is 0x00. Deleting that one line here leaves every
 * other assertion in the lab passing and the panel dark.
 */
#include <stdint.h>

#include "nrf54lm20a.h"

/* ── Board geometry ───────────────────────────────────────────────────────
 *
 * 20-pixel cells over the 240x536 panel: 12 columns, 26 rows (520 of 536 rows
 * used; the last 16 rows are the score strip and stay black).
 */
#define CELL   20u
#define COLS   (PANEL_W / CELL)   /* 12 */
#define ROWS   26u                /* 520 px of the 536 */

#define RGB565(r, g, b) \
    ((uint16_t)((((r) & 0xF8u) << 8) | (((g) & 0xFCu) << 3) | ((b) >> 3)))

#define COL_BG     RGB565(0, 0, 0)
#define COL_SNAKE  RGB565(0, 255, 96)
#define COL_HEAD   RGB565(160, 255, 160)
#define COL_FOOD   RGB565(255, 48, 64)

/*
 * EasyDMA reads its buffer over the bus, so it MUST live in RAM, not RRAM.
 * A string literal or a const array passed straight to DMA.TX.PTR sits in RRAM
 * and faults on silicon. This is the classic first-EasyDMA bug and a simulator
 * that permitted it would be modelling the part too leniently.
 *
 * Sized for one cell: 1 command byte + 20*20 RGB565 pixels.
 */
static uint8_t  dma_buf[1u + CELL * CELL * 2u];
static char     tx_buf[96];

/* ── UARTE ────────────────────────────────────────────────────────────────*/

static uint32_t str_copy(char *dst, const char *src)
{
    uint32_t n = 0;
    while (src[n] != '\0') {
        dst[n] = src[n];
        n++;
    }
    return n;
}

/* Eight lowercase hex digits, no prefix. Returns the count written. */
static uint32_t hex8(char *dst, uint32_t v)
{
    static const char digits[] = "0123456789abcdef";
    uint32_t i;
    for (i = 0; i < 8u; i++) {
        dst[i] = digits[(v >> (28u - 4u * i)) & 0xFu];
    }
    return 8u;
}

static void uarte_init(void)
{
    UARTE_PSEL_TXD(UARTE20_BASE) = UARTE20_PIN_TXD;
    UARTE_PSEL_RXD(UARTE20_BASE) = UARTE20_PIN_RXD;
    UARTE_BAUDRATE(UARTE20_BASE) = UARTE_BAUD_115200;
    UARTE_ENABLE(UARTE20_BASE)   = UARTE_ENABLE_UARTE;
}

static void uarte_write(const char *s)
{
    uint32_t n = str_copy(tx_buf, s);
    if (n == 0u) {
        return;
    }
    UARTE_EVENTS_DMA_TX_END(UARTE20_BASE) = 0u;
    UARTE_DMA_TX_PTR(UARTE20_BASE)        = (uint32_t)(uintptr_t)tx_buf;
    UARTE_DMA_TX_MAXCNT(UARTE20_BASE)     = n;
    UARTE_TASKS_DMA_TX_START(UARTE20_BASE) = 1u;
    while (UARTE_EVENTS_DMA_TX_END(UARTE20_BASE) == 0u) {
    }
    UARTE_EVENTS_DMA_TX_END(UARTE20_BASE) = 0u;
}

static void uarte_write_u32(const char *label, uint32_t v)
{
    char buf[32];
    uint32_t n = str_copy(buf, label);
    char digits[10];
    uint32_t d = 0;

    if (v == 0u) {
        digits[d++] = '0';
    }
    while (v > 0u) {
        digits[d++] = (char)('0' + (v % 10u));
        v /= 10u;
    }
    while (d > 0u) {
        buf[n++] = digits[--d];
    }
    buf[n++] = '\r';
    buf[n++] = '\n';
    buf[n] = '\0';
    uarte_write(buf);
}

/* ── SPIM22 + RM67162 ─────────────────────────────────────────────────────*/

static void spim_init(void)
{
    SPIM_PSEL_SCK(SPIM22_BASE)  = PANEL_PIN_SCK;
    SPIM_PSEL_MOSI(SPIM22_BASE) = PANEL_PIN_MOSI;
    SPIM_PSEL_MISO(SPIM22_BASE) = PANEL_PIN_MISO;
    /*
     * The two lines that make this an nRF54L driver rather than a ported
     * nRF52 one. With CSN and DCX selected, the controller owns chip select
     * and data/command; the firmware never touches either pin again.
     */
    SPIM_PSEL_CSN(SPIM22_BASE)  = PANEL_PIN_CSN;
    SPIM_PSEL_DCX(SPIM22_BASE)  = PANEL_PIN_DCX;

    SPIM_PRESCALER(SPIM22_BASE) = SPIM_PRESCALER_DIV2;
    SPIM_CONFIG(SPIM22_BASE)    = 0u;  /* MSB first, CPOL=0, CPHA=0 */
    SPIM_ENABLE(SPIM22_BASE)    = SPIM_ENABLE_SPIM;
}

/*
 * One command byte followed by `len` data bytes, as a SINGLE EasyDMA transfer.
 * DCXCNT = 1 tells the controller that exactly the first byte is a command.
 */
static void panel_xfer(uint32_t len)
{
    SPIM_EVENTS_END(SPIM22_BASE)   = 0u;
    SPIM_DCXCNT(SPIM22_BASE)       = 1u;
    SPIM_DMA_TX_PTR(SPIM22_BASE)   = (uint32_t)(uintptr_t)dma_buf;
    SPIM_DMA_TX_MAXCNT(SPIM22_BASE) = 1u + len;
    SPIM_TASKS_START(SPIM22_BASE)  = 1u;
    while (SPIM_EVENTS_END(SPIM22_BASE) == 0u) {
    }
    SPIM_EVENTS_END(SPIM22_BASE) = 0u;
}

static void panel_cmd(uint8_t cmd)
{
    dma_buf[0] = cmd;
    panel_xfer(0u);
}

static void panel_cmd1(uint8_t cmd, uint8_t p0)
{
    dma_buf[0] = cmd;
    dma_buf[1] = p0;
    panel_xfer(1u);
}

static void panel_window(uint16_t x0, uint16_t y0, uint16_t x1, uint16_t y1)
{
    dma_buf[0] = DCS_CASET;
    dma_buf[1] = (uint8_t)(x0 >> 8);
    dma_buf[2] = (uint8_t)(x0 & 0xFFu);
    dma_buf[3] = (uint8_t)(x1 >> 8);
    dma_buf[4] = (uint8_t)(x1 & 0xFFu);
    panel_xfer(4u);

    dma_buf[0] = DCS_RASET;
    dma_buf[1] = (uint8_t)(y0 >> 8);
    dma_buf[2] = (uint8_t)(y0 & 0xFFu);
    dma_buf[3] = (uint8_t)(y1 >> 8);
    dma_buf[4] = (uint8_t)(y1 & 0xFFu);
    panel_xfer(4u);
}

/* Fill one CELL x CELL cell with a solid colour. */
static void draw_cell(uint32_t col, uint32_t row, uint16_t colour)
{
    uint16_t x0 = (uint16_t)(col * CELL);
    uint16_t y0 = (uint16_t)(row * CELL);
    uint32_t i;

    panel_window(x0, y0, (uint16_t)(x0 + CELL - 1u), (uint16_t)(y0 + CELL - 1u));

    dma_buf[0] = DCS_RAMWR;
    for (i = 0; i < CELL * CELL; i++) {
        dma_buf[1u + 2u * i]      = (uint8_t)(colour >> 8);
        dma_buf[1u + 2u * i + 1u] = (uint8_t)(colour & 0xFFu);
    }
    panel_xfer(CELL * CELL * 2u);
}

static void panel_init(void)
{
    panel_cmd(DCS_SWRESET);
    panel_cmd(DCS_SLPOUT);
    panel_cmd1(DCS_COLMOD, COLMOD_RGB565);
    panel_cmd1(DCS_MADCTL, 0x00u);
    /*
     * NOT optional, and NOT a nicety. WRDISBV resets to 0x00 on this panel:
     * an AMOLED with brightness zero is black no matter what DISPON says.
     */
    panel_cmd1(DCS_WRDISBV, 0xFFu);
    panel_cmd(DCS_DISPON);
}

/* ── GPIO ─────────────────────────────────────────────────────────────────*/

static void gpio_init(void)
{
    /* LEDs: P1.22/25/27/28, active high. */
    GPIO_PIN_CNF(P1_BASE, LED0_PIN) = PIN_CNF_OUTPUT;
    GPIO_PIN_CNF(P1_BASE, LED1_PIN) = PIN_CNF_OUTPUT;
    GPIO_PIN_CNF(P1_BASE, LED2_PIN) = PIN_CNF_OUTPUT;
    GPIO_PIN_CNF(P1_BASE, LED3_PIN) = PIN_CNF_OUTPUT;
    GPIO_DIRSET(P1_BASE) = (1u << LED0_PIN) | (1u << LED1_PIN) |
                           (1u << LED2_PIN) | (1u << LED3_PIN);

    /*
     * Buttons: input with a pull-up, configured through PIN_CNF because that
     * is the ONLY register that carries the pull field. A driver that sets
     * direction through DIR alone leaves these pins floating, and a model that
     * decoded PIN_CNF at the wrong offset would read a stuck value forever.
     */
    GPIO_PIN_CNF(BTN_UP_PORT,    BTN_UP_PIN)    = PIN_CNF_INPUT_PULLUP;
    GPIO_PIN_CNF(BTN_DOWN_PORT,  BTN_DOWN_PIN)  = PIN_CNF_INPUT_PULLUP;
    GPIO_PIN_CNF(BTN_LEFT_PORT,  BTN_LEFT_PIN)  = PIN_CNF_INPUT_PULLUP;
    GPIO_PIN_CNF(BTN_RIGHT_PORT, BTN_RIGHT_PIN) = PIN_CNF_INPUT_PULLUP;
}

/* Active-low: a pressed button reads 0. */
static uint32_t button_pressed(uint32_t port, uint32_t pin)
{
    return (GPIO_IN(port) & (1u << pin)) == 0u ? 1u : 0u;
}

/* ── Map probe ────────────────────────────────────────────────────────────
 *
 * Prove the chip profile's map by TOUCHING it, and print what came back.
 *
 * This replaces a banner that printed `rram=2036K ram=511K` and
 * `p3@0x500D8600` as STRING LITERALS. Those read like assertions about the
 * silicon and were nothing of the kind: a literal is printed whatever the chip
 * profile says, so the smoke that asserted them passed with P3 deleted from
 * the chip entirely. Measured, not argued -- that deletion was tried and the
 * smoke still went 7/7.
 *
 * Each value below is READ BACK from the model:
 *
 *   rram  a word near the top of RRAM. On the sibling's 1524 KB map this
 *         address is outside the region.
 *   ram   write-then-read near the top of SRAM. Outside the sibling's 256 KB.
 *   p3    PIN_CNF on the FOURTH GPIO port, which the sibling does not have at
 *         all. Written and read back, so a missing port cannot answer.
 */
static void probe_map(void)
{
    char line[96];
    uint32_t n;
    uint32_t rram_word;
    uint32_t ram_word;
    uint32_t p3_word;

    /* RRAM is readable but never written by this firmware; the value does not
     * matter, reaching the address does. */
    rram_word = REG32(RRAM_PROBE_ADDR);

    REG32(RAM_PROBE_ADDR) = 0x54120000UL;  /* arbitrary marker */
    ram_word = REG32(RAM_PROBE_ADDR);

    /* P3.02 is the panel's chip-select pad. Configure it as an output through
     * PIN_CNF and read the value back out of the port. */
    GPIO_PIN_CNF(P3_BASE, 2) = PIN_CNF_OUTPUT;
    p3_word = GPIO_PIN_CNF(P3_BASE, 2);

    n = str_copy(line, "map rram=");
    n += hex8(line + n, rram_word);
    n += str_copy(line + n, " ram=");
    n += hex8(line + n, ram_word);
    n += str_copy(line + n, " p3cnf=");
    n += hex8(line + n, p3_word);
    line[n++] = '\r';
    line[n++] = '\n';
    line[n] = '\0';
    uarte_write(line);
}

/* ── Snake ────────────────────────────────────────────────────────────────*/

#define MAX_LEN 64u

enum { DIR_UP = 0, DIR_DOWN, DIR_LEFT, DIR_RIGHT };

static uint8_t snake_col[MAX_LEN];
static uint8_t snake_row[MAX_LEN];
static uint32_t snake_len;
static uint32_t dir;
static uint32_t score;
static uint8_t food_col, food_row;

/*
 * A tiny xorshift, seeded from a constant. Deterministic ON PURPOSE: a lab
 * whose food placement varied per run could not assert anything about the
 * frame it produces, and "the picture changed" would stop being evidence.
 */
static uint32_t rng_state = 0x54Cu;

static uint32_t rng_next(void)
{
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 17;
    rng_state ^= rng_state << 5;
    return rng_state;
}

static uint32_t cell_is_snake(uint32_t c, uint32_t r)
{
    uint32_t i;
    for (i = 0; i < snake_len; i++) {
        if (snake_col[i] == c && snake_row[i] == r) {
            return 1u;
        }
    }
    return 0u;
}

static void place_food(void)
{
    uint32_t tries = 0;
    do {
        food_col = (uint8_t)(rng_next() % COLS);
        food_row = (uint8_t)(rng_next() % ROWS);
        tries++;
    } while (cell_is_snake(food_col, food_row) && tries < 64u);
    draw_cell(food_col, food_row, COL_FOOD);
}

static void game_reset(void)
{
    uint32_t i;
    snake_len = 3u;
    for (i = 0; i < snake_len; i++) {
        snake_col[i] = (uint8_t)(COLS / 2u);
        snake_row[i] = (uint8_t)(ROWS / 2u + i);
    }
    dir = DIR_UP;
    score = 0;
    for (i = 0; i < snake_len; i++) {
        draw_cell(snake_col[i], snake_row[i], COL_SNAKE);
    }
    place_food();
}

/*
 * Buttons win when pressed. With none pressed the snake keeps its heading and
 * turns only to avoid a wall, so an unattended run still plays -- which is
 * what makes this a lab rather than a demo that dies in four moves. Press a
 * button (the lab drives them as stimuli) and it steers.
 */
static uint32_t read_input(void)
{
    if (button_pressed(BTN_UP_PORT, BTN_UP_PIN) && dir != DIR_DOWN) {
        dir = DIR_UP;
        return 1u;
    }
    if (button_pressed(BTN_DOWN_PORT, BTN_DOWN_PIN) && dir != DIR_UP) {
        dir = DIR_DOWN;
        return 1u;
    }
    if (button_pressed(BTN_LEFT_PORT, BTN_LEFT_PIN) && dir != DIR_RIGHT) {
        dir = DIR_LEFT;
        return 1u;
    }
    if (button_pressed(BTN_RIGHT_PORT, BTN_RIGHT_PIN) && dir != DIR_LEFT) {
        dir = DIR_RIGHT;
        return 1u;
    }
    return 0u;
}

/* Where does heading `d` put the head, from (c, r)? */
static void step_cell(uint32_t d, uint32_t c, uint32_t r, uint32_t *oc, uint32_t *or_)
{
    switch (d) {
    case DIR_UP:    *oc = c;      *or_ = r - 1u; break;
    case DIR_DOWN:  *oc = c;      *or_ = r + 1u; break;
    case DIR_LEFT:  *oc = c - 1u; *or_ = r;      break;
    default:        *oc = c + 1u; *or_ = r;      break;
    }
}

/*
 * Would this heading kill the snake -- wall OR its own body?
 *
 * The body half is not a nicety. Wall-avoidance alone walks the snake into the
 * corner, turns it back along its own neck and ends the game on the next move,
 * which is exactly what the first version of this firmware did: ten games in
 * 400 ticks, almost all scoring zero. A lab whose demo dies immediately paints
 * a handful of cells and proves very little about the panel.
 */
static uint32_t blocked(uint32_t d, uint32_t c, uint32_t r)
{
    uint32_t nc, nr;

    switch (d) {
    case DIR_UP:    if (r == 0u) { return 1u; } break;
    case DIR_DOWN:  if (r + 1u >= ROWS) { return 1u; } break;
    case DIR_LEFT:  if (c == 0u) { return 1u; } break;
    default:        if (c + 1u >= COLS) { return 1u; } break;
    }
    step_cell(d, c, r, &nc, &nr);
    return cell_is_snake(nc, nr);
}

static uint32_t abs_diff(uint32_t a, uint32_t b)
{
    return a > b ? a - b : b - a;
}

/*
 * With no button pressed, head for the food and refuse suicidal moves. The
 * result is a snake that actually plays: it eats, grows, and only dies when it
 * boxes itself in -- so an unattended run keeps painting the panel and keeps
 * producing score events for the lab to assert on. A pressed button always
 * wins; see read_input.
 */
static void autosteer(void)
{
    static const uint8_t order[4] = { DIR_UP, DIR_RIGHT, DIR_DOWN, DIR_LEFT };
    uint32_t c = snake_col[0];
    uint32_t r = snake_row[0];
    uint32_t best = 4u;
    uint32_t best_dist = 0xFFFFFFFFu;
    uint32_t i;

    for (i = 0; i < 4u; i++) {
        uint32_t d = order[i];
        uint32_t nc, nr, dist;

        if (blocked(d, c, r)) {
            continue;
        }
        step_cell(d, c, r, &nc, &nr);
        dist = abs_diff(nc, food_col) + abs_diff(nr, food_row);
        /* Prefer the current heading on a tie: straight lines look like a
         * snake, and a snake that jitters every tick is harder to eyeball. */
        if (dist < best_dist || (dist == best_dist && d == dir)) {
            best_dist = dist;
            best = d;
        }
    }
    if (best < 4u) {
        dir = best;
    }
}

int main(void)
{
    uint32_t tick;

    uarte_init();
    uarte_write("nRF54LM20A AMOLED Snake\r\n");
    probe_map();

    gpio_init();
    GPIO_OUTSET(P1_BASE) = (1u << LED0_PIN);

    spim_init();
    panel_init();
    uarte_write("panel init ok\r\n");

    game_reset();
    uarte_write("game start\r\n");

    for (tick = 0; tick < 400u; tick++) {
        uint32_t c, r, i;

        if (!read_input()) {
            autosteer();
        }

        /*
         * `blocked` is the ONE place a fatal move is decided, and it must be
         * consulted whoever chose the direction. autosteer already refuses
         * blocked headings, but a BUTTON can steer into a wall -- and the same
         * thing happens when nothing drives the buttons at all, because these
         * inputs are active-low and an undriven pin reads as held. Checking
         * only self-collision here let the head walk off the board and the row
         * counter wrap.
         */
        if (blocked(dir, snake_col[0], snake_row[0])) {
            uarte_write_u32("game over score=", score);
            /* Clear the board and start again so a long run keeps painting. */
            for (i = 0; i < snake_len; i++) {
                draw_cell(snake_col[i], snake_row[i], COL_BG);
            }
            draw_cell(food_col, food_row, COL_BG);
            game_reset();
            continue;
        }

        step_cell(dir, snake_col[0], snake_row[0], &c, &r);

        if (c == food_col && r == food_row) {
            score++;
            if (snake_len < MAX_LEN) {
                snake_len++;
            }
            uarte_write_u32("food score=", score);
            GPIO_OUTSET(P1_BASE) = (1u << LED1_PIN);
            place_food();
        } else {
            /* Erase the tail only when the snake did not grow. */
            draw_cell(snake_col[snake_len - 1u], snake_row[snake_len - 1u], COL_BG);
        }

        for (i = snake_len - 1u; i > 0u; i--) {
            snake_col[i] = snake_col[i - 1u];
            snake_row[i] = snake_row[i - 1u];
        }
        snake_col[0] = (uint8_t)c;
        snake_row[0] = (uint8_t)r;

        draw_cell(snake_col[0], snake_row[0], COL_HEAD);
        if (snake_len > 1u) {
            draw_cell(snake_col[1], snake_row[1], COL_SNAKE);
        }
    }

    uarte_write_u32("snake done score=", score);
    for (;;) {
    }
}
