/**
 * Minimal Arduino API shim for LabWired STM32F103 simulator.
 * Maps Arduino functions to STM32F103 register writes.
 */
#ifndef ARDUINO_H
#define ARDUINO_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Pin modes ---- */
#define INPUT       0
#define OUTPUT      1
#define INPUT_PULLUP 2

/* ---- Digital values ---- */
#define HIGH 1
#define LOW  0

/* ---- LED_BUILTIN ---- */
#define LED_BUILTIN 13  /* Maps to PA5 */

/* ---- STM32F103 Peripheral base addresses ---- */
#define RCC_BASE     0x40021000
#define GPIOA_BASE   0x40010800
#define GPIOB_BASE   0x40010C00
#define GPIOC_BASE   0x40011000
#define SYSTICK_BASE 0xE000E010
#define UART1_BASE   0x40013800
#define ADC1_BASE    0x40012400

/* Register access macros */
#define REG32(addr) (*(volatile uint32_t *)(addr))

/* RCC registers */
#define RCC_APB2ENR  REG32(RCC_BASE + 0x18)

/* GPIO registers (STM32F103 has CRL/CRH/IDR/ODR) */
#define GPIOA_CRL    REG32(GPIOA_BASE + 0x00)
#define GPIOA_CRH    REG32(GPIOA_BASE + 0x04)
#define GPIOA_IDR    REG32(GPIOA_BASE + 0x08)
#define GPIOA_ODR    REG32(GPIOA_BASE + 0x0C)
#define GPIOB_CRL    REG32(GPIOB_BASE + 0x00)
#define GPIOB_CRH    REG32(GPIOB_BASE + 0x04)
#define GPIOB_IDR    REG32(GPIOB_BASE + 0x08)
#define GPIOB_ODR    REG32(GPIOB_BASE + 0x0C)
#define GPIOC_CRL    REG32(GPIOC_BASE + 0x00)
#define GPIOC_CRH    REG32(GPIOC_BASE + 0x04)
#define GPIOC_IDR    REG32(GPIOC_BASE + 0x08)
#define GPIOC_ODR    REG32(GPIOC_BASE + 0x0C)

/* SysTick registers */
#define SYST_CSR     REG32(SYSTICK_BASE + 0x00)
#define SYST_RVR     REG32(SYSTICK_BASE + 0x04)
#define SYST_CVR     REG32(SYSTICK_BASE + 0x08)

/* UART registers */
#define UART1_SR     REG32(UART1_BASE + 0x00)
#define UART1_DR     REG32(UART1_BASE + 0x04)

/* ADC registers */
#define ADC1_SR      REG32(ADC1_BASE + 0x00)
#define ADC1_CR1     REG32(ADC1_BASE + 0x04)
#define ADC1_CR2     REG32(ADC1_BASE + 0x08)
#define ADC1_DR      REG32(ADC1_BASE + 0x4C)

/* ---- Internal helpers ---- */

static volatile uint32_t _millis_counter = 0;

static inline volatile uint32_t* _gpio_base(uint8_t pin) {
    if (pin < 16) return (volatile uint32_t*)GPIOA_BASE;
    if (pin < 32) return (volatile uint32_t*)GPIOB_BASE;
    return (volatile uint32_t*)GPIOC_BASE;
}

static inline uint8_t _gpio_pin(uint8_t pin) {
    return pin % 16;
}

/* ---- Arduino API implementation ---- */

static inline void pinMode(uint8_t pin, uint8_t mode) {
    /* Enable GPIO clocks (IOPAEN=bit2, IOPBEN=bit3, IOPCEN=bit4) */
    RCC_APB2ENR |= (1 << 2) | (1 << 3) | (1 << 4);

    volatile uint32_t *base = _gpio_base(pin);
    uint8_t p = _gpio_pin(pin);
    /* CRL for pins 0-7, CRH for pins 8-15 */
    volatile uint32_t *cr = (p < 8) ? (base + 0) : (base + 1);
    uint8_t shift = (p % 8) * 4;
    uint32_t mask = 0xF << shift;

    uint32_t val;
    if (mode == OUTPUT) {
        val = 0x1 << shift; /* Output push-pull, 10 MHz */
    } else if (mode == INPUT_PULLUP) {
        val = 0x8 << shift; /* Input with pull-up */
    } else {
        val = 0x4 << shift; /* Floating input */
    }
    *cr = (*cr & ~mask) | val;
}

static inline void digitalWrite(uint8_t pin, uint8_t val) {
    volatile uint32_t *base = _gpio_base(pin);
    volatile uint32_t *odr = base + 3; /* ODR offset = 0x0C / 4 */
    uint8_t p = _gpio_pin(pin);
    if (val) {
        *odr |= (1 << p);
    } else {
        *odr &= ~(1 << p);
    }
}

static inline int digitalRead(uint8_t pin) {
    volatile uint32_t *base = _gpio_base(pin);
    volatile uint32_t *idr = base + 2; /* IDR offset = 0x08 / 4 */
    uint8_t p = _gpio_pin(pin);
    return (*idr >> p) & 1;
}

static inline int analogRead(uint8_t pin) {
    (void)pin;
    /* Enable ADC1, start conversion, wait for EOC */
    ADC1_CR2 |= 1; /* ADON */
    ADC1_CR2 |= (1 << 30); /* SWSTART */
    /* Wait for EOC (bit 1 in SR) */
    while (!(ADC1_SR & (1 << 1))) {}
    return ADC1_DR & 0xFFF;
}

static inline void delay(uint32_t ms) {
    /* Simple busy-wait delay using SysTick countdown.
       Assume ~8MHz system clock for simulation. */
    for (uint32_t i = 0; i < ms; i++) {
        SYST_RVR = 8000 - 1; /* 1ms at 8MHz */
        SYST_CVR = 0;
        SYST_CSR = 0x5; /* Enable, use processor clock */
        while (!(SYST_CSR & (1 << 16))) {} /* Wait for COUNTFLAG */
        SYST_CSR = 0;
        _millis_counter++;
    }
}

static inline uint32_t millis(void) {
    return _millis_counter;
}

static inline void delayMicroseconds(uint32_t us) {
    /* Rough busy-wait: ~8 cycles per us at 8MHz */
    volatile uint32_t count = us * 2;
    while (count--) {}
}

/* ---- Serial class ---- */

typedef struct {
    int _dummy;
} SerialClass;

static inline void Serial_begin(uint32_t baud) {
    (void)baud;
    /* UART is pre-configured by the simulator */
}

static inline void Serial_print(const char *str) {
    while (*str) {
        while (!(UART1_SR & (1 << 7))) {} /* Wait for TXE */
        UART1_DR = *str++;
    }
}

static inline void Serial_println(const char *str) {
    Serial_print(str);
    Serial_print("\r\n");
}

static inline void Serial_print_int(int val) {
    char buf[12];
    int i = 0;
    if (val < 0) { Serial_print("-"); val = -val; }
    if (val == 0) { Serial_print("0"); return; }
    while (val > 0) { buf[i++] = '0' + (val % 10); val /= 10; }
    char out[12];
    for (int j = 0; j < i; j++) out[j] = buf[i - 1 - j];
    out[i] = '\0';
    Serial_print(out);
}

static inline void Serial_println_int(int val) {
    Serial_print_int(val);
    Serial_print("\r\n");
}

static inline int Serial_available(void) {
    return (UART1_SR & (1 << 5)) ? 1 : 0; /* RXNE */
}

static inline int Serial_read(void) {
    if (!Serial_available()) return -1;
    return UART1_DR & 0xFF;
}

/* Global Serial instance macros (C-compatible) */
#define Serial_begin(baud)       Serial_begin(baud)

/* ---- Tone ---- */
static inline void tone(uint8_t pin, unsigned int frequency) {
    (void)pin;
    (void)frequency;
    /* PWM tone: not yet supported in simulation */
}

static inline void noTone(uint8_t pin) {
    (void)pin;
}

/* ---- Map function ---- */
static inline long map(long x, long in_min, long in_max, long out_min, long out_max) {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
}

/* ---- constrain ---- */
#define constrain(amt,low,high) ((amt)<(low)?(low):((amt)>(high)?(high):(amt)))
#define min(a,b) ((a)<(b)?(a):(b))
#define max(a,b) ((a)>(b)?(a):(b))
#define abs(x) ((x)>0?(x):-(x))

#ifdef __cplusplus
}
#endif

#endif /* ARDUINO_H */
