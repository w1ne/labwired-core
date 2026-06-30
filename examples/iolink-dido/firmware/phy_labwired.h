/* IO-Link PHY backend bridging the iolinki stack to a simulated L476 USART. */
#ifndef PHY_LABWIRED_H
#define PHY_LABWIRED_H

#include "iolinki/phy.h"
#include <stdint.h>

void iolink_phy_labwired_init(iolink_phy_api_t *phy, uintptr_t usart_base);

#endif /* PHY_LABWIRED_H */
