/* IO-Link MASTER PHY backend bridging iolinki-master to the simulated L476
 * USART2. Provides the PHY ops plus a ready-to-use master config whose
 * checked/flush/prepare/wake-up callbacks drive the same USART2. */
#ifndef PHY_LABWIRED_H
#define PHY_LABWIRED_H

#include "iolinki_master/master.h"

const iolink_phy_api_t *phy_labwired_master_phy(void);
iolink_master_config_t phy_labwired_master_config(void);

#endif /* PHY_LABWIRED_H */
