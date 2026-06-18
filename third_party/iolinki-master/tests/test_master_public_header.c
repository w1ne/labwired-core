#include "iolinki_master/master.h"

#include <string.h>

#ifdef IOLINK_H
#error "iolinki_master/master.h must not include the aggregate iolinki/iolink.h device header"
#endif

int main(void)
{
    iolink_master_port_t port;
    iolink_master_controller_t controller;
    iolink_master_tick_event_t event = IOLINK_MASTER_TICK_CYCLE_DUE;
    iolink_master_result_t result = IOLINK_MASTER_STATUS_OK;

    memset(&port, 0, sizeof(port));
    memset(&controller, 0, sizeof(controller));
    (void)iolink_master_validate_phy_contract(NULL, NULL);
    (void)iolink_master_tick_at(&port, event, 0U);
    (void)iolink_master_get_next_tick_time(&port, 0U, NULL);
    (void)iolink_master_controller_tick_at(&controller, 0U);
    (void)iolink_master_controller_get_next_tick_time(&controller, 0U, NULL);
    (void)result;

    if(IOLINK_MASTER_STATUS_PENDING != 1)
    {
        return 1;
    }

    if(IOLINK_MASTER_ERR_CHECKSUM != -3)
    {
        return 2;
    }

    if(IOLINK_MASTER_PORT_STORAGE_SIZE > IOLINK_MASTER_PORT_STORAGE_BUDGET_SIZE)
    {
        return 3;
    }

    if(IOLINK_MASTER_CONTROLLER_STORAGE_SIZE > IOLINK_MASTER_CONTROLLER_STORAGE_BUDGET_SIZE)
    {
        return 4;
    }

    return 0;
}
