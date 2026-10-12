#include <stdint.h>
#include "cpu.h"

#include "def-helper.h"

DEF_HELPER_3(warn_cp_invalid_el, void, env, ptr, i32);
DEF_HELPER_3(warn_cp_invalid_access, void, env, ptr, i32);

#include "def-helper.h"
