// LabWired - PlatformIO + LabWired integration example
// Minimal nRF52840 UART0 TX driver.
//
// LabWired's nRF52840 model streams every byte written to UART0 TXD to the
// host process's stdout in real time. That is the entire bridge that lets
// PlatformIO's Unity test runner read results back out of the simulator.
#ifndef NRF_UART_H
#define NRF_UART_H

void uart_init(void);
void uart_putc(char c);
void uart_puts(const char *s);

#endif // NRF_UART_H
