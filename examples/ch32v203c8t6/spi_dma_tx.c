/* SPI2 TX on DMA1 channel 5 smoke for the CH32V203C8T6.
 *
 * ch32fun supplies the QingKe V4 startup and interrupt vector table. SPI2
 * is a 16-bit master; its TX request feeds DMA1 channel 5 (the RM's
 * hard-wired SPI2_TX channel) in circular half-word mode with HTIE and
 * TCIE. As in ch32fun's spidmatest, CTLR2.TXDMAEN is set before the
 * channel is enabled, so the request is already pending at enable time.
 *
 * The channel 5 handler (PFIC interrupt 31) logs 'H' on HT and 'T' on TC
 * and disables the channel after the third lap. main() then prints the log
 * and the channel's reloaded CNTR on USART1 TX.
 */
#include "ch32fun.h"
#include <stdint.h>

#define FRAME_COUNT 8u
#define LAPS 3u

static uint16_t frames[FRAME_COUNT] = {
    0x0AAF, 0x1234, 0x5678, 0x9ABC, 0xDEF0, 0x0F0F, 0xF0F0, 0xA55A,
};
static volatile char events[2u * LAPS];
static volatile uint32_t event_count;
static volatile uint32_t laps;

__attribute__((interrupt)) void DMA1_Channel5_IRQHandler(void) {
    uint32_t intfr = DMA1->INTFR;
    DMA1->INTFCR = DMA1_IT_GL5;
    if ((intfr & DMA1_IT_HT5) != 0 && event_count < 2u * LAPS) {
        events[event_count++] = 'H';
    }
    if ((intfr & DMA1_IT_TC5) != 0) {
        if (event_count < 2u * LAPS) {
            events[event_count++] = 'T';
        }
        if (++laps >= LAPS) {
            DMA1_Channel5->CFGR &= ~DMA_CFGR1_EN;
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

static void uart_putu(uint32_t value) {
    char digits[10];
    uint32_t n = 0;
    do {
        digits[n++] = (char)('0' + value % 10u);
        value /= 10u;
    } while (value != 0u);
    while (n != 0u) {
        uart_putc(digits[--n]);
    }
}

int main(void) {
    SystemInit();
    funGpioInitAll();

    /* USART1 console: PA9 TX at 144 MHz / 115200. */
    funPinMode(PA9, GPIO_CFGLR_OUT_10Mhz_AF_PP);
    RCC->APB2PCENR |= RCC_APB2Periph_USART1;
    USART1->BRR = 0x04E2u;
    USART1->CTLR1 = USART_Mode_Tx | CTLR1_UE_Set;

    /* SPI2 pads: PB13 SCK, PB15 MOSI. */
    funPinMode(PB13, GPIO_CFGLR_OUT_50Mhz_AF_PP);
    funPinMode(PB15, GPIO_CFGLR_OUT_50Mhz_AF_PP);
    RCC->APB1PCENR |= RCC_APB1Periph_SPI2;
    RCC->AHBPCENR |= RCC_AHBPeriph_DMA1;

    SPI2->CTLR1 = SPI_NSS_Soft | SPI_CPHA_1Edge | SPI_CPOL_Low |
                  SPI_DataSize_16b | SPI_Mode_Master |
                  SPI_Direction_1Line_Tx | SPI_BaudRatePrescaler_8;
    SPI2->CTLR1 |= CTLR1_SPE_Set;
    SPI2->CTLR2 = SPI_CTLR2_TXDMAEN;

    /* The channel is programmed while disabled, then enabled. */
    DMA1_Channel5->PADDR = (uint32_t)&SPI2->DATAR;
    DMA1_Channel5->MADDR = (uint32_t)frames;
    DMA1_Channel5->CNTR = FRAME_COUNT;
    DMA1_Channel5->CFGR = DMA_M2M_Disable | DMA_Priority_VeryHigh |
                          DMA_MemoryDataSize_HalfWord |
                          DMA_PeripheralDataSize_HalfWord |
                          DMA_MemoryInc_Enable | DMA_Mode_Circular |
                          DMA_DIR_PeripheralDST | DMA_IT_TC | DMA_IT_HT;
    NVIC_EnableIRQ(DMA1_Channel5_IRQn);
    DMA1_Channel5->CFGR |= DMA_CFGR1_EN;

    for (volatile uint32_t timeout = 0; timeout < 200000u && laps < LAPS;
         ++timeout) {
        __asm__ volatile("nop");
    }

    uart_puts("SPI2_DMA events=");
    for (uint32_t i = 0; i < event_count; ++i) {
        uart_putc(events[i]);
    }
    uart_puts(" laps=");
    uart_putu(laps);
    uart_puts(" cntr=");
    uart_putu(DMA1_Channel5->CNTR);
    uart_puts("\r\n");

    while ((USART1->STATR & USART_FLAG_TC) == 0) {
    }
    for (;;) {
    }
}
