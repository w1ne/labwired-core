/* USART1 RXNE interrupt smoke for the CH32V203C8T6.
 *
 * ch32fun supplies the QingKe V4 startup and interrupt vector table. The
 * simulator injects four bytes on USART1 RX; IRQ 53 must read DATAR in the
 * handler before main prints those bytes back on USART1 TX.
 */
#include "ch32fun.h"
#include <stdint.h>

#define RX_COUNT 4u

static volatile uint8_t received[RX_COUNT];
static volatile uint32_t received_count;

__attribute__((interrupt)) void USART1_IRQHandler(void) {
    if ((USART1->STATR & USART_FLAG_RXNE) != 0) {
        uint8_t byte = (uint8_t)USART1->DATAR;
        uint32_t index = received_count;
        if (index < RX_COUNT) {
            received[index] = byte;
            received_count = index + 1;
        }
    }
}

static void uart_putc(char byte) {
    while ((USART1->STATR & USART_FLAG_TXE) == 0) {
    }
    USART1->DATAR = (uint8_t)byte;
}

static void uart_puts(const char *text) {
    while (*text != '\0') {
        uart_putc(*text++);
    }
}

static int received_is_wch(void) {
    return received_count == RX_COUNT && received[0] == 'w' &&
           received[1] == 'c' && received[2] == 'h' && received[3] == '!';
}

int main(void) {
    SystemInit();
    funGpioInitAll();

    /* CH32V203 default USART1 pads: PA9 TX, PA10 RX. */
    funPinMode(PA9, GPIO_CFGLR_OUT_10Mhz_AF_PP);
    funPinMode(PA10, GPIO_CFGLR_IN_FLOAT);
    RCC->APB2PCENR |= RCC_APB2Periph_USART1;

    /* SystemInit selects the 144 MHz HSI x18 clock: 144 MHz / 115200. */
    USART1->BRR = 0x04E2u;
    USART1->CTLR1 = USART_Mode_Rx | USART_Mode_Tx |
                    USART_CTLR1_RXNEIE | CTLR1_UE_Set;
    NVIC_EnableIRQ(USART1_IRQn);

    for (volatile uint32_t timeout = 0;
         timeout < 10000u && received_count < RX_COUNT;
         ++timeout) {
        __asm__ volatile("nop");
    }

    if (received_is_wch()) {
        uart_puts("USART1_IRQ_RX=");
        for (uint32_t i = 0; i < RX_COUNT; ++i) {
            uart_putc((char)received[i]);
        }
        uart_puts("\r\n");
    } else {
        uart_puts("USART1_IRQ_TIMEOUT\r\n");
    }

    while ((USART1->STATR & USART_FLAG_TC) == 0) {
    }
    for (;;) {
    }
}
