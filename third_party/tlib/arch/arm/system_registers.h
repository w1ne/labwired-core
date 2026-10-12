/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include "system_registers_common.h"

/* To enable banking of coprocessor registers depending on ns-bit we
 * add a bit to distinguish between secure and non-secure cpregs in the
 * hashtable.
 */
#define CP_REG_NS_SHIFT 29
#define CP_REG_NS_MASK  (1 << CP_REG_NS_SHIFT)

#define ENCODE_CP_REG(cp, is64, ns, crn, crm, opc1, opc2) \
    ((ns) << CP_REG_NS_SHIFT | ((cp) << 16) | ((is64) << 15) | ((crn) << 11) | ((crm) << 7) | ((opc1) << 3) | (opc2))

//  Cover the entire register width, when used on an instruction address field (crm, opc1, opc2)
#define ANY 0xFF
