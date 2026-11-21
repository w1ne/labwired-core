/*
 * Pomodoro Pal - firmware for the CH32V003F4P6 (heypcb.ai merch 02), on ch32fun.
 *
 * The Pomodoro technique, on a tomato:
 *   IDLE   the 12 o'clock LED breathes. Tap the gold pad in the calyx to start.
 *   FOCUS  25 minutes. All 12 red LEDs light; one goes out every 125 s, clockwise from 12 o'clock;
 *          the next one to go out breathes, so a glance tells you where you are.
 *   PAUSE  tap during focus or a break: the lit LEDs blink slowly. Tap again to carry on.
 *   BREAK  a soft three-note chirp, then the two green leaf tips breathe for 5 minutes (15 minutes
 *          after every 4th focus); the ring counts the break down the same way. Two chirps at the end,
 *          then IDLE again: you start the next focus yourself.
 *   Hold the pad for 2 s at any time: back to IDLE, the session count cleared.
 *
 * Pins (from the board's netlist; KiCad symbol MCU_WCH_RiscV:CH32V003FxPx)
 *   minute LEDs 0..11 (12 o'clock, clockwise): PD3 PD2 PC7 PC6 PC5 PC4 PC3 PC2 PC1 PC0 PD0 PD4, high = on
 *   PD6 LEAF (both green leaf tips), PA1 BUZZ (piezo through 100 ohm), PA2 TOUCH (ADC channel 0,
 *   through 1k to the gold pad), PD1 SWIO, PD5 and PD7 (NRST) unused.
 *
 * Power: 5 V from USB-C, about 40 mA with every LED lit (12 x ~3 mA through 1k), well inside any port.
 * Build: see README.md (ch32fun + riscv-none-elf-gcc).
 */
#include "ch32fun.h"
#include "ch32v00x_touch.h"

#define FOCUS_MS (25UL * 60UL * 1000UL)
#define BREAK_MS (5UL * 60UL * 1000UL)
#define LONG_BREAK_MS (15UL * 60UL * 1000UL)
#define HOLD_RESET_MS 2000UL
#define TOUCH_ITER 3                 /* ADC samples per touch read (ch32fun's ReadTouchPin) */
#define TOUCH_EVERY_MS 20UL

static const uint8_t ring[12] = { PD3, PD2, PC7, PC6, PC5, PC4, PC3, PC2, PC1, PC0, PD0, PD4 };
#define LEAF PD6
#define BUZZ PA1

enum { IDLE, FOCUS, BREAK } state = IDLE;
static uint8_t paused;
static uint8_t sessions;            /* focus sessions finished since the last reset */
static uint32_t phase_ms;           /* time spent in this FOCUS or BREAK, pauses excluded */
static uint32_t phase_len;

volatile uint32_t ms;
void SysTick_Handler(void) __attribute__((interrupt));
void SysTick_Handler(void)
{
    SysTick->CMP += DELAY_MS_TIME;
    SysTick->SR = 0;
    ms++;
}

static void systick_init(void)
{
    SysTick->CTLR = 0;
    SysTick->CMP = DELAY_MS_TIME - 1;
    SysTick->CNT = 0;
    SysTick->CTLR = SYSTICK_CTLR_STE | SYSTICK_CTLR_STIE | SYSTICK_CTLR_STCLK;
    NVIC_EnableIRQ(SysTick_IRQn);
}

/* ---- the piezo: a square wave from the pin, blocking (a chirp is a few hundred ms) ---- */
static void tone(uint32_t hz, uint32_t dur_ms)
{
    uint32_t half_us = 500000UL / hz;
    uint32_t cycles = hz * dur_ms / 1000UL;
    for (uint32_t i = 0; i < cycles; i++) {
        funDigitalWrite(BUZZ, FUN_HIGH);
        Delay_Us(half_us);
        funDigitalWrite(BUZZ, FUN_LOW);
        Delay_Us(half_us);
    }
}

static void chirp(int notes)
{
    static const uint16_t up[3] = { 2637, 3136, 3951 };    /* E7 G7 B7: near the piezo's 4 kHz peak */
    for (int i = 0; i < notes; i++) {
        tone(up[i % 3], 90);
        Delay_Ms(60);
    }
}

/* ---- touch: one reading, and a baseline that follows slow drift while the pad is untouched ---- */
static uint32_t baseline;

static uint32_t touch_raw(void) { return ReadTouchPin(GPIOA, 2, 0, TOUCH_ITER); }

static int touched(void)
{
    uint32_t v = touch_raw();
    uint32_t thresh = baseline / 16 + 20;                  /* tune on hardware */
    if (v > baseline + thresh)
        return 1;
    baseline = (baseline * 15 + v) / 16;                   /* untouched: follow the drift */
    return 0;
}

/* ---- LEDs ---- */
/* A soft-PWM phase, 0..31, from the 48 MHz SysTick counter: about 1.5 kHz, no visible flicker. */
static uint8_t pwm_phase(void) { return (uint8_t)((SysTick->CNT >> 10) & 31); }

/* The ring empties clockwise from 12 o'clock: the `lit` LEDs still on are indices 12-lit .. 11, and the
 * one about to go out (`breathing_idx`) breathes at `level` (0..32 of 32). */
static void ring_show(int lit, int breathing_idx, uint8_t level)
{
    uint8_t pwm = pwm_phase();
    for (int k = 0; k < 12; k++) {
        int on = (k >= 12 - lit);
        if (k == breathing_idx)
            on = pwm < level;
        funDigitalWrite(ring[k], on ? FUN_HIGH : FUN_LOW);
    }
}

static uint8_t breath(uint32_t period_ms)
{
    uint32_t t = ms % period_ms;
    uint32_t half = period_ms / 2;
    uint32_t v = (t < half) ? t : (period_ms - t);         /* triangle 0..half */
    return (uint8_t)(2 + (v * 30) / half);                 /* 2..32 of 32 */
}

static void start(int st)
{
    state = st;
    paused = 0;
    phase_ms = 0;
    if (st == FOCUS)
        phase_len = FOCUS_MS;
    else if (st == BREAK)
        phase_len = (sessions % 4 == 0) ? LONG_BREAK_MS : BREAK_MS;
}

int main(void)
{
    SystemInit();
    funGpioInitAll();
    RCC->APB2PCENR |= RCC_APB2Periph_ADC1;
    for (int k = 0; k < 12; k++) {
        funPinMode(ring[k], GPIO_Speed_10MHz | GPIO_CNF_OUT_PP);
        funDigitalWrite(ring[k], FUN_LOW);
    }
    funPinMode(LEAF, GPIO_Speed_10MHz | GPIO_CNF_OUT_PP);
    funPinMode(BUZZ, GPIO_Speed_10MHz | GPIO_CNF_OUT_PP);
    funPinMode(PD5, GPIO_CNF_IN_PUPD);                     /* unused: pulled, never floating */
    funDigitalWrite(PD5, FUN_HIGH);
    systick_init();
    InitTouchADC();

    for (int i = 0; i < 16; i++)
        baseline += touch_raw();
    baseline /= 16;
    chirp(1);                                              /* hello */

    uint32_t last = ms, last_touch = 0, held = 0;
    uint8_t was_touched = 0;
    for (;;) {
        uint32_t now = ms;
        uint32_t dt = now - last;
        last = now;
        if (!paused && state != IDLE)
            phase_ms += dt;

        /* touch: a tap toggles, a 2 s hold resets */
        if (now - last_touch >= TOUCH_EVERY_MS) {
            last_touch = now;
            int t = touched();
            if (t) {
                held += TOUCH_EVERY_MS;
                if (held >= HOLD_RESET_MS && held < HOLD_RESET_MS + TOUCH_EVERY_MS) {
                    sessions = 0;
                    start(IDLE);
                    funDigitalWrite(LEAF, FUN_LOW);
                    chirp(1);
                }
            } else {
                if (was_touched && held < HOLD_RESET_MS) {
                    if (state == IDLE)
                        start(FOCUS);
                    else
                        paused = !paused;
                }
                held = 0;
            }
            was_touched = (uint8_t)t;
        }

        /* the phase clock */
        if (state != IDLE && phase_ms >= phase_len) {
            if (state == FOCUS) {
                sessions++;
                ring_show(0, -1, 0);
                chirp(3);
                start(BREAK);
            } else {
                funDigitalWrite(LEAF, FUN_LOW);
                chirp(2);
                start(IDLE);
            }
        }

        /* what the face shows */
        if (state == IDLE) {
            ring_show(0, 0, breath(3000));
            funDigitalWrite(LEAF, FUN_LOW);
        } else {
            uint32_t step = phase_len / 12;
            int lit = 12 - (int)(phase_ms / step);          /* 12 .. 1 */
            if (lit < 1)
                lit = 1;
            if (paused) {
                ring_show(((now / 600) & 1) ? lit : 0, -1, 0);
            } else {
                ring_show(lit, 12 - lit, breath(1600));
            }
            if (state == BREAK)
                funDigitalWrite(LEAF, (pwm_phase() < breath(4000)) ? FUN_HIGH : FUN_LOW);
        }
    }
}
