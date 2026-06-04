/* IO-Link PHY backend bridging the iolinki stack to the simulated L476 USART2. */
#ifndef PHY_LABWIRED_H
#define PHY_LABWIRED_H

#include "iolinki/phy.h"

const iolink_phy_api_t *iolink_phy_labwired_get(void);

#endif /* PHY_LABWIRED_H */
