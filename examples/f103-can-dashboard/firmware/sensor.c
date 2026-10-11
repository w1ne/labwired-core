/* SENSOR node: temperature on CAN ID 0x200 every 100 ms, status LED on PC13.
 *
 * CAN_BITRATE is the one number this node's bit timing hangs on. The bus runs
 * at 500 kbit/s. Build with -DCAN_BITRATE=250000 for the field bug (BRP 7
 * instead of 3), or 500000 for the fix.
 *
 * The temperature is a SYNTHETIC value, not a sensor reading: a slow triangle
 * wave from 21.0 to 23.0 degC, sent as degC x 10, big-endian in 2 bytes. (The
 * F103 internal temperature sensor is not modelled as a physical quantity in
 * the simulator, so a fake reading is the honest choice.)
 *
 * ABOM is off (MCR.ABOM = 0), as in a lot of field firmware: once the
 * controller goes bus-off it stays bus-off until the firmware restarts it.
 *
 * LED: slow blink (1 Hz) while the CAN controller is healthy, fast blink
 * (5 Hz) while ESR.BOFF is set. USART1 once per second: TEC/REC and state.
 */
#include "common.h"

#ifndef CAN_BITRATE
#define CAN_BITRATE 500000u
#endif
_Static_assert(CAN_PCLK1_HZ % (CAN_BITRATE * CAN_TQ_PER_BIT) == 0u,
               "CAN_BITRATE must divide 36 MHz / 18 tq exactly");

#define CAN_ID_TEMP 0x200u
#define TICK_MS 100u

static volatile uint32_t g_tick; /* 100 ms ticks since boot */

void TIM3_IRQHandler(void)
{
    TIM_SR(TIM3_BASE) = 0u;
    g_tick++;
}

static uint32_t fake_temp_x10(uint32_t tick)
{
    uint32_t phase = tick % 40u; /* 4 s triangle */
    uint32_t up = phase < 20u ? phase : 40u - phase;
    return 210u + up; /* 21.0 .. 23.0 degC */
}

static const char *state_name(uint32_t esr)
{
    if ((esr & CAN_ESR_BOFF) != 0u) {
        return "BUS_OFF";
    }
    if ((esr & CAN_ESR_EPVF) != 0u) {
        return "PASSIVE";
    }
    return "ACTIVE";
}

static void report(uint32_t esr)
{
    uart_puts("SENSOR bitrate=");
    uart_put_uint(CAN_BITRATE);
    uart_puts(" state=");
    uart_puts(state_name(esr));
    uart_puts(" tec=");
    uart_put_uint((esr >> 16) & 0xFFu);
    uart_puts(" rec=");
    uart_put_uint(esr >> 24);
    uart_puts("\r\n");
}

int main(void)
{
    clock_init_72mhz();
    uart_init_115200();
    RCC_APB2ENR |= RCC_APB2ENR_IOPCEN;
    gpio_crh_set(GPIOC_BASE, 13u, CNF_OUT_PP_2MHZ);
    can_init(CAN_BTR_FOR(CAN_BITRATE), 0xFFFFFFFFu, 0xFFFFFFFFu);

    RCC_APB1ENR |= RCC_APB1ENR_TIM3EN;
    timer_start_100us(TIM3_BASE, TICK_MS * 10u);
    nvic_enable(IRQ_TIM3);

    uint32_t seen = 0u;
    for (;;) {
        uint32_t now = g_tick;
        if (now == seen) {
            __WFI(); /* TIM3 wakes us; a spurious wake just loops */
            continue;
        }
        seen = now;
        uint32_t t = fake_temp_x10(now);
        can_send(CAN_ID_TEMP, ((t >> 8) & 0xFFu) | ((t & 0xFFu) << 8), 0u, 2u);

        uint32_t esr = CAN_ESR;
        uint32_t half_period = (esr & CAN_ESR_BOFF) != 0u ? 1u : 5u;
        if (now % half_period == 0u) {
            GPIO_ODR(GPIOC_BASE) ^= (1u << 13);
        }
        if (now % 10u == 0u) {
            report(esr);
        }
    }
}
