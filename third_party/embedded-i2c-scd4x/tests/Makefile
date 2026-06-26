
# use the second argument of make as argument to control test bed initialization
TEST_ARG := $(wordlist 2, 2, $(MAKECMDGOALS))
# ...turn it into do-nothing target
$(eval $(TEST_ARG):;@:)

  ifeq ($(TEST_ARG), mux)
    driver_dir := ..
    mux_dir := ../i2c-mux-testbed
    i2c_mux_sources = ${mux_dir}/i2c_mux.h ${mux_dir}/i2c_mux.c
    macros := "-D USE_MUX"
  else
    mux_dir := .
    i2c_mux_sources =
    macros :=
  endif

driver_dir := ..
common_sources = $(driver_dir)/sensirion_config.h $(driver_dir)/sensirion_common.h $(driver_dir)/sensirion_common.c
i2c_sources = $(driver_dir)/sensirion_i2c_hal.h ${driver_dir}/sensirion_i2c.h $(driver_dir)/sensirion_i2c.c
sensirion_test_sources = sensirion_test_setup.cpp $(i2c_mux_sources)

sw_i2c_dir := $(driver_dir)/sample-implementations/GPIO_bit_banging

hw_i2c_impl_src = $(driver_dir)/sample-implementations/linux_user_space/sensirion_i2c_hal.c
sw_i2c_impl_src = $(sw_i2c_dir)/sample-implementations/linux_user_space/sensirion_i2c_gpio.c $(sw_i2c_dir)/sensirion_i2c_hal.c

scd4x_sources = $(driver_dir)/scd4x_i2c.h $(driver_dir)/scd4x_i2c.c

CXXFLAGS ?= $(CFLAGS) -fsanitize=address -I$(mux_dir) -I$(driver_dir) -I$(sw_i2c_dir) ${macros}
ifdef CI
	CXXFLAGS += -Werror
endif
LDFLAGS ?= -lasan -lstdc++ -lCppUTest -lCppUTestExt

.PHONY: clean test


scd4x_test_binaries := scd4x_test_hw_i2c scd4x_test_sw_i2c

all: $(scd4x_test_binaries)

scd4x_test_hw_i2c: scd4x_i2c_test.cpp $(scd4x_sources) $(sensirion_test_sources) $(i2c_sources) $(hw_i2c_impl_src) $(common_sources)
	$(CXX) $(CXXFLAGS) -o $@ $^ $(LDFLAGS)

scd4x_test_sw_i2c: scd4x_i2c_test.cpp $(scd4x_sources) $(sensirion_test_sources) $(i2c_sources) $(sw_i2c_impl_src) $(common_sources)
	$(CXX) $(CXXFLAGS) -o $@ $^ $(LDFLAGS)

test: $(scd4x_test_binaries)
	set -ex; for test in $(scd4x_test_binaries); do echo $${test}; ./$${test}; echo; done;

clean:
	$(RM) $(scd4x_test_binaries)
