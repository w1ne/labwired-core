/*
 * ARM cpu parameters.
 *
 * Copyright (c) 2003 Fabrice Bellard
 * SPDX-License-Identifier: LGPL-2.0+
 */

#pragma once

#ifdef TARGET_ARMV7

//  TODO: TGAU: #93583 Verify 32 vs 40 when implementing LPAE
#define TARGET_PHYS_ADDR_SPACE_BITS 32
#define TARGET_PAGE_BITS            10  //  1k
#define NB_MMU_MODES                4

#else

#ifdef TARGET_AARCH64
#define TARGET_PHYS_ADDR_SPACE_BITS 52
#else
#define TARGET_PHYS_ADDR_SPACE_BITS 40
#endif

#define TARGET_PAGE_BITS 12  //  4k
#define NB_MMU_MODES     15
#endif
