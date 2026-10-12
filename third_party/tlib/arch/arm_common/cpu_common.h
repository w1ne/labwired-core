/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

/* For M profile, some registers are banked secure vs non-secure;
 * these are represented as a 2-element array where the first element
 * is the non-secure copy and the second is the secure copy.
 * When the CPU does not have implement the security extension then
 * only the first element is used.
 * This means that the copy for the current security state can be
 * accessed via env->registerfield[env->secure] (whether the security
 * extension is implemented or not).
 */
enum {
    M_REG_NS = 0,
    M_REG_S = 1,
    M_REG_NUM_BANKS = 2,
};

/* Some register fields aren't banked between security states.
 * We decide to store them in secure version of the register.
 * When accessing shared fields, prefer using M_REG_COMMON to signify they are shared.
 * WARNING: our implementation depends on the fact that common fields are stored in the Secure bank.
 */
#define M_REG_COMMON M_REG_S

#define MAKE_64BIT_MASK(shift, length) (((~0ULL) >> (64 - (length))) << (shift))
