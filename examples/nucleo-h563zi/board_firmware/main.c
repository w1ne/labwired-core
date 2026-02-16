// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#include <stdint.h>

#include "stm32h563xx.h"

#define LED1_PIN 0U
#define LED2_PIN 4U
#define LED3_PIN 4U
#define BTN_PIN 13U

#define USART3_BRR_115200_AT_64MHZ 556U

void __libc_init_array(void) {}

static void delay_cycles(uint32_t cycles) {
  while (cycles-- > 0U) {
    __asm volatile("nop");
  }
}

static void gpio_config_output(GPIO_TypeDef *gpio, uint32_t pin) {
  const uint32_t shift = pin * 2U;
  const uint32_t mask = 0x3UL << shift;

  gpio->MODER = (gpio->MODER & ~mask) | (0x1UL << shift);
  gpio->OTYPER &= ~(1UL << pin);
  gpio->OSPEEDR = (gpio->OSPEEDR & ~mask) | (0x1UL << shift);
  gpio->PUPDR &= ~mask;
}

static void gpio_config_input_pullup(GPIO_TypeDef *gpio, uint32_t pin) {
  const uint32_t shift = pin * 2U;
  const uint32_t mask = 0x3UL << shift;

  gpio->MODER &= ~mask;
  gpio->PUPDR = (gpio->PUPDR & ~mask) | (0x1UL << shift);
}

static void uart3_init(void) {
  RCC->AHB2ENR |= RCC_AHB2ENR_GPIODEN;
  RCC->APB1LENR |= RCC_APB1LENR_USART3EN;
  RCC->CCIPR1 &= ~RCC_CCIPR1_USART3SEL;

  GPIOD->MODER &= ~(GPIO_MODER_MODE8_Msk | GPIO_MODER_MODE9_Msk);
  GPIOD->MODER |= GPIO_MODER_MODE8_1 | GPIO_MODER_MODE9_1;

  GPIOD->OTYPER &= ~(GPIO_OTYPER_OT8 | GPIO_OTYPER_OT9);

  GPIOD->OSPEEDR |= GPIO_OSPEEDR_OSPEED8_Msk | GPIO_OSPEEDR_OSPEED9_Msk;

  GPIOD->PUPDR &= ~(GPIO_PUPDR_PUPD8_Msk | GPIO_PUPDR_PUPD9_Msk);
  GPIOD->PUPDR |= GPIO_PUPDR_PUPD9_0;

  GPIOD->AFR[1] &= ~(GPIO_AFRH_AFSEL8_Msk | GPIO_AFRH_AFSEL9_Msk);
  GPIOD->AFR[1] |= (7UL << GPIO_AFRH_AFSEL8_Pos) | (7UL << GPIO_AFRH_AFSEL9_Pos);

  USART3->CR1 = 0U;
  USART3->CR2 = 0U;
  USART3->CR3 = 0U;
  USART3->BRR = USART3_BRR_115200_AT_64MHZ;
  USART3->CR1 = USART_CR1_TE | USART_CR1_RE | USART_CR1_UE;
}

static void uart3_write_byte(uint8_t byte) {
  while ((USART3->ISR & USART_ISR_TXE_TXFNF) == 0U) {
  }
  USART3->TDR = byte;
}

static void uart3_write_str(const char *s) {
  while (*s != '\0') {
    uart3_write_byte((uint8_t)*s);
    s++;
  }
}

static void uart3_write_u32_dec(uint32_t value) {
  char buf[10];
  uint32_t i = 0U;

  if (value == 0U) {
    uart3_write_byte((uint8_t)'0');
    return;
  }

  while (value > 0U && i < (uint32_t)sizeof(buf)) {
    buf[i++] = (char)('0' + (value % 10U));
    value /= 10U;
  }

  while (i > 0U) {
    i--;
    uart3_write_byte((uint8_t)buf[i]);
  }
}

static void led_write(uint32_t on) {
  GPIOB->BSRR = on ? (1UL << LED1_PIN) : (1UL << (LED1_PIN + 16U));
  GPIOF->BSRR = on ? (1UL << LED2_PIN) : (1UL << (LED2_PIN + 16U));
  GPIOG->BSRR = on ? (1UL << LED3_PIN) : (1UL << (LED3_PIN + 16U));
}

int main(void) {
  RCC->AHB2ENR |= RCC_AHB2ENR_GPIOBEN | RCC_AHB2ENR_GPIOCEN | RCC_AHB2ENR_GPIOFEN | RCC_AHB2ENR_GPIOGEN;

  gpio_config_output(GPIOB, LED1_PIN);
  gpio_config_output(GPIOF, LED2_PIN);
  gpio_config_output(GPIOG, LED3_PIN);
  gpio_config_input_pullup(GPIOC, BTN_PIN);

  uart3_init();

  uart3_write_str("H563-BLINK-UART\r\n");

  uint32_t blink_count = 0U;
  uint32_t led_on = 0U;

  while (1) {
    led_on ^= 1U;
    led_write(led_on);

    const uint32_t btn = (GPIOC->IDR >> BTN_PIN) & 1U;

    uart3_write_str("BLINK ");
    uart3_write_u32_dec(blink_count);
    uart3_write_str(" PB0=");
    uart3_write_u32_dec(led_on);
    uart3_write_str(" PF4=");
    uart3_write_u32_dec(led_on);
    uart3_write_str(" PG4=");
    uart3_write_u32_dec(led_on);
    uart3_write_str(" BTN13=");
    uart3_write_u32_dec(btn);
    uart3_write_str("\r\n");

    blink_count++;
    delay_cycles(12000000U);
  }
}
