// LabWired - PlatformIO + LabWired integration example
// Unity test suite executed inside the LabWired nRF52840 model.
//
// PlatformIO builds this into firmware.elf and hands the ELF to the
// `test_testing_command` (see platformio.ini), which runs it in LabWired.
// Unity output is streamed over UART0 to stdout, where PlatformIO's Unity
// parser reads the PASS/FAIL results back.
#include <unity.h>
#include "nrf_uart.h"

// ---------------------------------------------------------------------------
// Unity output transport.
//
// PlatformIO's bundled unity_config.h wires Unity's output macros to these
// weakly-declared hooks. Implementing them here sends every Unity character
// out of UART0 -> LabWired stdout -> PlatformIO.
// ---------------------------------------------------------------------------
void unittest_uart_begin(void) { uart_init(); }
void unittest_uart_putchar(char c) { uart_putc(c); }
void unittest_uart_flush(void) {}
void unittest_uart_end(void) {}

void setUp(void) {}
void tearDown(void) {}

static int add(int a, int b) { return a + b; }

static void test_addition(void) {
    TEST_ASSERT_EQUAL_INT(7, add(3, 4));
}

static void test_uart_is_enabled(void) {
    // UART0 ENABLE must read back as 4 after uart_init(), proving the model's
    // peripheral register actually latched the write.
    volatile unsigned int *enable = (volatile unsigned int *)0x40002500u;
    TEST_ASSERT_EQUAL_UINT(4u, *enable);
}

static void test_string_length(void) {
    const char *s = "labwired";
    int n = 0;
    while (s[n]) {
        n++;
    }
    TEST_ASSERT_EQUAL_INT(8, n);
}

int main(void) {
    UNITY_BEGIN();
    RUN_TEST(test_addition);
    RUN_TEST(test_uart_is_enabled);
    RUN_TEST(test_string_length);
    return UNITY_END();
}
