#pragma once

#include <stdint.h>

int32_t tlib_set_pending_interrupt(int32_t interruptNo, int32_t level);
void tlib_set_little_endian_mode(bool mode);
