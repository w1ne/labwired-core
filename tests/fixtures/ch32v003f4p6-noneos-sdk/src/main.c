#include "debug.h"
int main(void)
{
    SystemCoreClockUpdate();
    Delay_Init();
    USART_Printf_Init(115200);
    printf("CH32V003_BOOT clk=%lu\r\n", (unsigned long)SystemCoreClock);
    GPIO_InitTypeDef g = {0};
    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOC, ENABLE);
    g.GPIO_Pin = GPIO_Pin_0;
    g.GPIO_Mode = GPIO_Mode_Out_PP;
    g.GPIO_Speed = GPIO_Speed_30MHz;
    GPIO_Init(GPIOC, &g);
    for (int i = 0; i < 4; i++) {
        GPIO_WriteBit(GPIOC, GPIO_Pin_0, (i & 1) ? Bit_SET : Bit_RESET);
        Delay_Ms(10);
        printf("tick %d\r\n", i);
    }
    printf("CH32V003_DONE\r\n");
    while (1) {}
}
