#include <stdint.h>
#include "stm32h563xx.h"

// Stress test parameters
#define STRESS_BUFFER_SIZE 256U
uint8_t stress_buffer[STRESS_BUFFER_SIZE];

void uart3_init(void) {
    RCC->AHB2ENR |= RCC_AHB2ENR_GPIODEN;
    RCC->APB1LENR |= RCC_APB1LENR_USART3EN;
    
    // Config PD8 (TX) as AF7
    GPIOD->MODER &= ~GPIO_MODER_MODE8_Msk;
    GPIOD->MODER |= GPIO_MODER_MODE8_1;
    GPIOD->AFR[1] &= ~GPIO_AFRH_AFSEL8_Msk;
    GPIOD->AFR[1] |= (7UL << GPIO_AFRH_AFSEL8_Pos);

    USART3->BRR = 556U; // 115200 at 64MHz
    USART3->CR3 |= USART_CR3_DMAT; // Enable DMA for transmission
    USART3->CR1 = USART_CR1_TE | USART_CR1_UE;
}

void gpdma_init(void) {
    RCC->AHB1ENR |= RCC_AHB1ENR_GPDMA1EN;

    // GPDMA1 channel 0: memory -> USART3 TDR, byte elements, source
    // increments, destination fixed. Software request streams the block
    // without TXE pacing (sim stress flow); real-silicon UART TX would
    // select the usart3_tx hardware request instead.
    GPDMA1_Channel0->CTR1 = DMA_CTR1_SINC;
    GPDMA1_Channel0->CTR2 = DMA_CTR2_SWREQ;
    GPDMA1_Channel0->CBR1 = STRESS_BUFFER_SIZE;
    GPDMA1_Channel0->CSAR = (uint32_t)stress_buffer;
    GPDMA1_Channel0->CDAR = (uint32_t)&USART3->TDR;
    GPDMA1_Channel0->CCR  = DMA_CCR_EN;
}

void uart3_write_str(const char *s) {
    while (*s != '\0') {
        while (!(USART3->ISR & USART_ISR_TXE_TXFNF));
        USART3->TDR = (uint8_t)*s++;
    }
}

int main(void) {
    // Fill buffer with test pattern
    for(uint32_t i=0; i<STRESS_BUFFER_SIZE; i++) {
        stress_buffer[i] = (uint8_t)(i & 0xFF);
    }

    uart3_init();
    gpdma_init();

    // Signal start of stress test
    GPIOB->BSRR = (1UL << 0); // LED Green ON
    uart3_write_str("HIL Stress Test Started\r\n");

    // Wait for GPDMA transfer complete (C0SR.TCF)
    while(!(GPDMA1_Channel0->CSR & DMA_CSR_TCF)) {
        // High-stress loop: if this takes more cycles than expected (regression), 
        // LabWired will catch it.
    }

    uart3_write_str("HIL Stress Test Passed\r\n");
    GPIOB->BSRR = (1UL << 16); // LED Green OFF
    
    // Halt to allow LabWired to collect final metrics
    __asm volatile("bkpt #0");

    while(1);
}
