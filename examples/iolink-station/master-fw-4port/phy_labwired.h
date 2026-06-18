/* 4-port IO-Link MASTER PHY: one USART per port (USART2/3/4/5 on the L476).
 * Fills caller-owned phy/config arrays for iolink_master_controller_init. */
#ifndef PHY_LABWIRED_H
#define PHY_LABWIRED_H

#include "iolinki_master/master.h"

#define LW_MASTER_PORTS 4

void phy_labwired_fill(iolink_phy_api_t phys[LW_MASTER_PORTS],
                       iolink_master_config_t cfgs[LW_MASTER_PORTS]);

#endif /* PHY_LABWIRED_H */
