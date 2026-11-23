#include "debug.h"

/* Stock noneos-sdk style: the handler relies on the QingKe hardware
 * prologue/epilogue (HPE) to save caller-saved registers. */
void USART1_IRQHandler(void) __attribute__((interrupt("WCH-Interrupt-fast")));

static volatile uint8_t rx_buf[8];
static volatile uint32_t rx_len;

void USART1_IRQHandler(void)
{
    if (USART_GetITStatus(USART1, USART_IT_RXNE) != RESET) {
        uint8_t c = (uint8_t)USART_ReceiveData(USART1);
        if (rx_len < sizeof(rx_buf)) {
            rx_buf[rx_len++] = c;
        }
    }
}

int main(void)
{
    SystemCoreClockUpdate();
    Delay_Init();

    GPIO_InitTypeDef gpio = {0};
    USART_InitTypeDef usart = {0};
    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOD | RCC_APB2Periph_USART1, ENABLE);
    /* Default USART1 mapping: TX PD5, RX PD6. */
    gpio.GPIO_Pin = GPIO_Pin_5;
    gpio.GPIO_Speed = GPIO_Speed_30MHz;
    gpio.GPIO_Mode = GPIO_Mode_AF_PP;
    GPIO_Init(GPIOD, &gpio);
    gpio.GPIO_Pin = GPIO_Pin_6;
    gpio.GPIO_Mode = GPIO_Mode_IN_FLOATING;
    GPIO_Init(GPIOD, &gpio);

    usart.USART_BaudRate = 115200;
    usart.USART_WordLength = USART_WordLength_8b;
    usart.USART_StopBits = USART_StopBits_1;
    usart.USART_Parity = USART_Parity_No;
    usart.USART_HardwareFlowControl = USART_HardwareFlowControl_None;
    usart.USART_Mode = USART_Mode_Tx | USART_Mode_Rx;
    USART_Init(USART1, &usart);
    USART_ITConfig(USART1, USART_IT_RXNE, ENABLE);
    NVIC_EnableIRQ(USART1_IRQn);
    USART_Cmd(USART1, ENABLE);

    /* Bytes already queued on RX interrupt this printf: the handler's
     * calls clobber ra and a0-a5 inside the SDK printf path. */
    printf("USART_IRQ_BOOT\r\n");
    while (rx_len < 4) {}
    printf("USART_IRQ_RX %c%c%c%c\r\n", rx_buf[0], rx_buf[1], rx_buf[2], rx_buf[3]);
    printf("USART_IRQ_DONE\r\n");
    while (1) {}
}
