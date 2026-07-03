/* IO-Link PHY backend bridging the iolinki stack to a simulated L476 USART. */
#ifndef PHY_LABWIRED_H
#define PHY_LABWIRED_H

#include "iolinki/phy.h"
#include <stdint.h>

const iolink_phy_api_t *iolink_phy_labwired_get(void);

#endif /* PHY_LABWIRED_H */
