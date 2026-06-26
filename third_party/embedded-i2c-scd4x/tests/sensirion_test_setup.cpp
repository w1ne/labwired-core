#include "sensirion_test_setup.h"
#include "CppUTest/CommandLineTestRunner.h"
#include "sensirion_i2c_hal.h"

#ifdef USE_MUX
#include "i2c_mux.h"
#define INIT_TESTBED(x, y) sensirion_i2c_mux_set_single_channel((x), (y))
#else
#define INIT_TESTBED(x, y) 0
#endif

#define MUX_CHANNEL 0x71

int main(int argc, char** argv) {
    sensirion_i2c_hal_init();
    int16_t error = INIT_TESTBED(MUX_CHANNEL, 1);
    CHECK_EQUAL_ZERO_TEXT(error, "test-bed initialization failed");
    int result = CommandLineTestRunner::RunAllTests(argc, argv);
    sensirion_i2c_hal_free();
    return result;
}
