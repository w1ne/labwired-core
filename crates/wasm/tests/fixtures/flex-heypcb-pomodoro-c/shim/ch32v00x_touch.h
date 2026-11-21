/* SPDX-License-Identifier: MIT
 *
 * The two touch calls that firmware/pomodoro.c uses. See flex_ch32fun.c.
 */
#ifndef FLEX_CH32V00X_TOUCH_H
#define FLEX_CH32V00X_TOUCH_H

#include "ch32fun.h"

void InitTouchADC(void);
uint32_t ReadTouchPin(GPIO_TypeDef *io, int portpin, int adcno, int iterations);

#endif
