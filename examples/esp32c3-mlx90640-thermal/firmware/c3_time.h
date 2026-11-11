#ifndef C3_TIME_H
#define C3_TIME_H

#include <stdint.h>

/* SYSTIMER UNIT0 runs at 16 MHz independently of the CPU clock. */
void c3_time_init(void);
uint64_t c3_time_us(void);
void c3_sleep_us(uint32_t delay_us);

#endif
