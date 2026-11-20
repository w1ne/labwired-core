/*
 * Nexus Design AI / LabWired: real STM32F103C8 bare-metal firmware fixture.
 * No RTOS, mock HAL, host-side state machine, or motor driver model.
 *
 * Contract: PA10 USART1_RX accepts "S,<sequence>,<distance_mm>\n" at 115200 8N1;
 * PB0 MOTOR_ENABLE is always low on boot and after a fault.
 * Test-only operator START/RESET events use PA0/PA1 digital button inputs.
 * They MUST be mapped to a separate authenticated control channel in production.
 */
#include <stdint.h>
#include <stdbool.h>

#define REG32(addr) (*(volatile uint32_t *)(addr))
#define RCC_APB2ENR REG32(0x40021018u)
#define GPIOA_CRL   REG32(0x40010800u)
#define GPIOA_CRH   REG32(0x40010804u)
#define GPIOA_IDR   REG32(0x40010808u)
#define GPIOA_ODR   REG32(0x4001080Cu)
#define GPIOB_CRL   REG32(0x40010C00u)
#define GPIOB_BSRR  REG32(0x40010C10u)
#define USART1_SR   REG32(0x40013800u)
#define USART1_DR   REG32(0x40013804u)
#define USART1_BRR  REG32(0x40013808u)
#define USART1_CR1  REG32(0x4001380Cu)
#define SYST_CSR    REG32(0xE000E010u)
#define SYST_RVR    REG32(0xE000E014u)
#define SYST_CVR    REG32(0xE000E018u)
#define RXNE (1u << 5)
#define TXE (1u << 7)
#define START_PIN (1u << 0)
#define RESET_PIN (1u << 1)
#define MOTOR_PIN (1u << 0)
#define FRESHNESS_MS 100u

typedef enum { BOOT_LOW, ARMED, RUNNING, FAULT_LATCHED } State;
static volatile State state;
static uint32_t now_ms, last_fresh_ms, last_seq;
static uint8_t qualification, last_start, last_reset;
static char line[32];
static uint8_t line_len, discard_frame;

static void tx_char(char c) {
    while (!(USART1_SR & TXE)) {}
    USART1_DR = (uint8_t)c;
}
static void tx(const char *s) { while (*s) tx_char(*s++); }
static void tx_u32(uint32_t x) {
    char digits[10];
    unsigned n = 0;
    do { digits[n++] = (char)('0' + (x % 10u)); x /= 10u; } while (x);
    while (n) tx_char(digits[--n]);
}
static void trace(const char *event) {
    tx(event); tx(" ms="); tx_u32(now_ms);
    tx(" state="); tx_u32((uint32_t)state);
    tx(" PB0="); tx_u32(state == RUNNING ? 1u : 0u);
    tx("\n");
}
static void set_motor(bool on) {
    /* BSRR high half clears, low half sets: independent of output mode. */
    GPIOB_BSRR = on ? MOTOR_PIN : (MOTOR_PIN << 16);
}
static void clear_state(void) {
    set_motor(false);
    state = BOOT_LOW;
    qualification = 0;
    last_seq = 0;
    last_fresh_ms = now_ms;
    line_len = 0;
    discard_frame = 0;
    trace("BOOT_LOW");
}
static void fault(void) {
    set_motor(false);
    state = FAULT_LATCHED;
    trace("FAULT_LATCHED");
}

/* Strictly decimal, bounded, complete frames; no unsafe or partial sequence. */
static bool number(const char **s, uint32_t *out) {
    uint32_t n = 0;
    bool any = false;
    while (**s >= '0' && **s <= '9') {
        uint32_t d = (uint32_t)(*(*s)++ - '0');
        if (n > (UINT32_MAX - d) / 10u) return false;
        n = n * 10u + d;
        any = true;
    }
    *out = n;
    return any;
}
static void parse_frame(void) {
    const char *p = line;
    uint32_t seq, distance;
    if (*p++ != 'S' || *p++ != ',' || !number(&p, &seq) ||
        *p++ != ',' || !number(&p, &distance) || *p != '\0' ||
        seq == 0 || seq <= last_seq) {
        return; /* Malformed, duplicate and old sequences do NOT refresh. */
    }
    last_seq = seq;
    last_fresh_ms = now_ms;
    if (distance < 500u) {
        qualification = 0;
        if (state == RUNNING) fault();
        else if (state == ARMED) state = BOOT_LOW;
        return;
    }
    if (state == BOOT_LOW && qualification < 3u) {
        ++qualification;
        if (qualification == 3u) {
            state = ARMED;
            trace("ARMED");
        }
    }
    tx("FRESH seq="); tx_u32(seq);
    tx(" ms="); tx_u32(last_fresh_ms);
    tx("\n");
}
static void poll_rx(void) {
    if (!(USART1_SR & RXNE)) return;
    char c = (char)USART1_DR;
    if (c == '\n') {
        if (!discard_frame) {
            line[line_len] = '\0';
            parse_frame();
        }
        line_len = 0;
        discard_frame = 0;
    } else if (c == '\r' && !discard_frame) {
        /* Optional CR before LF. */
    } else if (!discard_frame && (uint8_t)c >= 32u && (uint8_t)c < 127u &&
               line_len < (uint8_t)(sizeof(line) - 1u)) {
        line[line_len++] = c;
    } else {
        /* Never recover within an invalid line: discard through LF. */
        discard_frame = 1;
    }
}
static void poll_buttons(void) {
    uint32_t inputs = GPIOA_IDR;
    bool reset = (inputs & RESET_PIN) != 0;
    bool start = (inputs & START_PIN) != 0;
    if (reset && !last_reset) {
        clear_state(); /* Manual reset requires three NEW frames and START. */
    }
    last_reset = (uint8_t)reset;
    if (start && !last_start && state == ARMED &&
        (uint32_t)(now_ms - last_fresh_ms) < FRESHNESS_MS) {
        state = RUNNING;
        set_motor(true);
        trace("RUNNING");
    }
    last_start = (uint8_t)start;
}

int main(void) {
    /* Real STM32F1 clock gates: AFIO, GPIOA, GPIOB, USART1. Reset HSI = 8 MHz. */
    RCC_APB2ENR |= (1u << 0) | (1u << 2) | (1u << 3) | (1u << 14);
    /* PA0/PA1: input pull-down, PA9 AF push-pull, PA10 floating RX. */
    GPIOA_CRL = (GPIOA_CRL & ~0xFFu) | 0x88u;
    GPIOA_ODR &= ~(START_PIN | RESET_PIN);
    GPIOA_CRH = (GPIOA_CRH & ~(0xFFu << 4)) | (0x4Bu << 4);
    /* PB0: 2 MHz GPIO push-pull, low before motor can be enabled. */
    set_motor(false);
    GPIOB_CRL = (GPIOB_CRL & ~0xFu) | 0x2u;
    USART1_BRR = 0x45u; /* 8 MHz / 115200, 8N1 default CR2/CR3. */
    USART1_CR1 = (1u << 13) | (1u << 3) | (1u << 2);
    SYST_RVR = 7999u; /* 1 ms tick at 8 MHz. */
    SYST_CVR = 0u;
    SYST_CSR = 5u; /* processor clock / no SysTick interrupt */
    now_ms = 0;
    last_start = 0;
    last_reset = 0;
    clear_state();
    for (;;) {
        if (SYST_CSR & (1u << 16)) ++now_ms; /* COUNTFLAG */
        poll_rx();
        poll_buttons();
        if (state == RUNNING &&
            (uint32_t)(now_ms - last_fresh_ms) >= FRESHNESS_MS) fault();
    }
}
