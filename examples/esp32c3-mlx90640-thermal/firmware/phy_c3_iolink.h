/* IO-Link PHY backend over the simulated ESP32-C3 UART1 (the C/Q line).
 *
 * Bridges the LabWired generic UART model at 0x60010000 (UART1, Stm32F1
 * register layout) to the iolinki `iolink_phy_api_t`. UART0 (0x60000000) stays
 * the human debug console; UART1 carries the IO-Link byte stream between this
 * device's DLL and the native IO-Link master attached to uart1 in the sim. */
#ifndef PHY_C3_IOLINK_H
#define PHY_C3_IOLINK_H

#include "iolinki/phy.h"

/* Return the PHY API vtable backed by the C3 UART1. */
const iolink_phy_api_t *iolink_phy_c3_get(void);

#endif /* PHY_C3_IOLINK_H */
