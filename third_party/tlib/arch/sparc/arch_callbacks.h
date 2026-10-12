#pragma once

#include <stdint.h>

int32_t tlib_find_best_interrupt(void);
void tlib_acknowledge_interrupt(int32_t number);
void tlib_on_cpu_halted(void);
void tlib_on_cpu_power_down(void);
