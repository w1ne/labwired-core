// LabWired - PlatformIO + LabWired integration example
// Custom Unity configuration for a no-framework (bare-metal) build.
//
// PlatformIO discovers this file by walking the test hierarchy. It routes
// Unity's output to the UART transport implemented in test_main.c, which in
// turn writes to nRF52840 UART0 -> LabWired stdout.
#ifndef UNITY_CONFIG_H
#define UNITY_CONFIG_H

#ifdef __cplusplus
extern "C" {
#endif

void unittest_uart_begin(void);
void unittest_uart_putchar(char c);
void unittest_uart_flush(void);
void unittest_uart_end(void);

#define UNITY_OUTPUT_START()    unittest_uart_begin()
#define UNITY_OUTPUT_CHAR(c)    unittest_uart_putchar(c)
#define UNITY_OUTPUT_FLUSH()    unittest_uart_flush()
#define UNITY_OUTPUT_COMPLETE() unittest_uart_end()

#ifdef __cplusplus
}
#endif

#endif // UNITY_CONFIG_H
