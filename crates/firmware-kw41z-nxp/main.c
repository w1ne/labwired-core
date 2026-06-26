/*
 * KW41Z (MKW41Z4) NXP-clock-bring-up firmware fixture for the LabWired emulator.
 *
 * Boot path:
 *   Reset_Handler (vendor startup_MKW41Z4.S)
 *     -> SystemInit            (vendor system_MKW41Z4.c)
 *     -> main                  (this file)
 *          -> BOARD_BootClockRUN  (vendor clock_config.c, verbatim FRDM-KW41Z)
 *                -> BOARD_RfOscInit         (RSIM RF osc enable + RF_OSC_READY spin)
 *                -> CLOCK_SetSimSafeDivs     (fsl_clock.c)
 *                -> CLOCK_InitOsc0           (fsl_clock.c)
 *                -> CLOCK_BootToFeeMode      (fsl_clock.c, spins on MCG_S)
 *                -> CLOCK_SetInternalRefClkConfig (fsl_clock.c)
 *                -> CLOCK_SetSimConfig       (fsl_clock.c)
 *          -> LPUART0 setup + LPUART_WriteBlocking (vendor fsl_lpuart.c)
 *
 * This file (main.c) is the ONLY hand-written C; every register-poking clock
 * routine below the BOARD_BootClockRUN call is genuine, unmodified NXP code.
 */

#include "fsl_device_registers.h"
#include "fsl_clock.h"
#include "fsl_lpuart.h"
#include "clock_config.h"

/* After BOARD_BootClockRUN the 32 MHz RF crystal feeds OSCERCLK, which we
 * select as the LPUART0 clock source. So the LPUART baud divider sees 32 MHz. */
#define DEMO_LPUART        LPUART0
#define DEMO_LPUART_CLK_HZ 32000000U
#define DEMO_LPUART_SRC    2U /* SIM_SOPT2[LPUART0SRC] = 0b10 = OSCERCLK */

static const char kBanner[] = "KW41Z_NXP_OK\n";

int main(void)
{
    lpuart_config_t config;

    /* ---- Genuine NXP clock bring-up (fsl_clock.c via clock_config.c) ---- */
    BOARD_BootClockRUN();

    /* ---- LPUART0 clocking: source = OSCERCLK, gate PORTC + LPUART0 ---- */
    CLOCK_SetLpuartClock(DEMO_LPUART_SRC);
    CLOCK_EnableClock(kCLOCK_PortC);

    /* PORTC PCR6 = LPUART0_RX (ALT4), PCR7 = LPUART0_TX (ALT4) */
    PORTC->PCR[6] = PORT_PCR_MUX(4U);
    PORTC->PCR[7] = PORT_PCR_MUX(4U);

    /* ---- LPUART0 via the real NXP fsl_lpuart.c driver ---- */
    LPUART_GetDefaultConfig(&config);
    config.baudRate_Bps = 115200U;
    config.enableTx     = true;
    config.enableRx     = false;

    (void)LPUART_Init(DEMO_LPUART, &config, DEMO_LPUART_CLK_HZ);

    LPUART_WriteBlocking(DEMO_LPUART, (const uint8_t *)kBanner, sizeof(kBanner) - 1U);

    for (;;)
    {
        __NOP();
    }
}
