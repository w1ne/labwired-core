/* SPDX-License-Identifier: MIT
 *
 * The part of the ch32fun API that firmware/pomodoro.c uses, for a build of
 * that file as portable LabWired Flex firmware. This is not ch32fun and has
 * no ch32fun code: pins are Flex ports, time is virtual time, and the only
 * registers are a SysTick counter and compare value held in memory.
 * flex_ch32fun.c has the implementation and states each assumption.
 */
#ifndef FLEX_CH32FUN_H
#define FLEX_CH32FUN_H

#include <stdint.h>
#include "funconfig.h"

#ifndef FUNCONF_SYSTICK_USE_HCLK
#define FUNCONF_SYSTICK_USE_HCLK 0
#endif
#define FUNCONF_SYSTEM_CORE_CLOCK 48000000

/* A pin is port index * 16 + bit, with ports A, B, C, D as 0, 1, 2, 3. */
#define FLEX_PIN(port, bit) ((port) * 16 + (bit))
enum {
    PA0 = FLEX_PIN(0, 0), PA1, PA2, PA3, PA4, PA5, PA6, PA7,
    PC0 = FLEX_PIN(2, 0), PC1, PC2, PC3, PC4, PC5, PC6, PC7,
    PD0 = FLEX_PIN(3, 0), PD1, PD2, PD3, PD4, PD5, PD6, PD7,
};
#define FLEX_PIN_COUNT 64

#define FUN_LOW 0
#define FUN_HIGH 1
#define GPIO_Speed_10MHz 1
#define GPIO_CNF_OUT_PP 0x00
#define GPIO_CNF_IN_PUPD 0x08

typedef struct {
    uint8_t index;
} GPIO_TypeDef;
extern GPIO_TypeDef flex_gpio[4];
#define GPIOA (&flex_gpio[0])
#define GPIOC (&flex_gpio[2])
#define GPIOD (&flex_gpio[3])

typedef struct {
    volatile uint32_t APB2PCENR;
} RCC_TypeDef;
extern RCC_TypeDef flex_rcc;
#define RCC (&flex_rcc)
#define RCC_APB2Periph_ADC1 (1u << 9)

typedef struct {
    volatile uint32_t CTLR;
    volatile uint32_t SR;
    volatile uint32_t CNT;
    uint32_t RESERVED0;
    volatile uint32_t CMP;
} SysTick_Type;
/* Each use of `SysTick` outside the SysTick handler takes virtual time. */
SysTick_Type *flex_systick(void);
#define SysTick (flex_systick())
#define SYSTICK_CTLR_STE (1u << 0)
#define SYSTICK_CTLR_STIE (1u << 1)
#define SYSTICK_CTLR_STCLK (1u << 2)
#define DELAY_US_TIME (FUNCONF_SYSTEM_CORE_CLOCK / (FUNCONF_SYSTICK_USE_HCLK ? 1000000 : 8000000))
#define DELAY_MS_TIME (DELAY_US_TIME * 1000)

#define SysTick_IRQn 12
void NVIC_EnableIRQ(int irq);

void SystemInit(void);
void funGpioInitAll(void);
void funPinMode(int pin, int mode);
void funDigitalWrite(int pin, int level);
void Delay_Us(uint32_t us);
#define Delay_Ms(ms) Delay_Us((uint32_t)(ms) * 1000u)

#endif
