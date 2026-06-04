// LabWired - PlatformIO + LabWired integration example
// Default application entry point.
//
// This is a *weak* main so that `pio run` (build the application) links and
// also runs in the simulator. When building tests (`pio test`), the strong
// main() in test/test_smoke/test_main.c overrides it.
#include "nrf_uart.h"

__attribute__((weak)) int main(void) {
    uart_init();
    uart_puts("nrf52840 application boot via LabWired\n");
    while (1) {
    }
    return 0;
}
