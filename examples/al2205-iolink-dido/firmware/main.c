/* AL2205-style IO-Link DI device — firmware-under-test.
 * Milestone 1: prove boot + debug UART. The IO-Link stack is added in M2+. */
#include "debug_uart.h"

int main(void) {
    dbg_uart_init();
    dbg_puts("AL2205 BOOT\r\n");
    for (;;) {
    }
}
