/* Common softmmu definitions and inline routines.  */

/* XXX: find something cleaner.
 * Furthermore, this is false for 64 bits targets
 */
#define ldul_user       ldl_user
#define ldul_kernel     ldl_kernel
#define ldul_hypv       ldl_hypv
#define ldul_executive  ldl_executive
#define ldul_supervisor ldl_supervisor

#include "softmmu_defs.h"

#define ACCESS_TYPE 0
#define MEMSUFFIX   MMU_MODE0_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX

#define ACCESS_TYPE 1
#define MEMSUFFIX   MMU_MODE1_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX

//  All architectures also have the '_data'-suffixed MMU mode (with NB_MMU_MODES index)
//  so ACCESS_TYPE ranges from 0 to NB_MMU_MODES which gives NB_MMU_MODES+1 TLB tables.
#if (NB_MMU_MODES >= 3)

#define ACCESS_TYPE 2
#define MEMSUFFIX   MMU_MODE2_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 3) */

#if (NB_MMU_MODES >= 4)

#define ACCESS_TYPE 3
#define MEMSUFFIX   MMU_MODE3_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 4) */

#if (NB_MMU_MODES >= 5)

#define ACCESS_TYPE 4
#define MEMSUFFIX   MMU_MODE4_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 5) */

#if (NB_MMU_MODES >= 6)

#define ACCESS_TYPE 5
#define MEMSUFFIX   MMU_MODE5_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 6) */

#if (NB_MMU_MODES >= 7)

#define ACCESS_TYPE 6
#define MEMSUFFIX   MMU_MODE6_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 7) */

#if (NB_MMU_MODES >= 8)

#define ACCESS_TYPE 7
#define MEMSUFFIX   MMU_MODE7_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 8) */

#if (NB_MMU_MODES >= 9)

#define ACCESS_TYPE 8
#define MEMSUFFIX   MMU_MODE8_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 9) */

#if (NB_MMU_MODES >= 10)

#define ACCESS_TYPE 9
#define MEMSUFFIX   MMU_MODE9_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 10) */

#if (NB_MMU_MODES >= 11)

#define ACCESS_TYPE 10
#define MEMSUFFIX   MMU_MODE10_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 11) */

#if (NB_MMU_MODES >= 12)

#define ACCESS_TYPE 11
#define MEMSUFFIX   MMU_MODE11_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 12) */

#if (NB_MMU_MODES >= 13)

#define ACCESS_TYPE 12
#define MEMSUFFIX   MMU_MODE12_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 13) */

#if (NB_MMU_MODES >= 14)

#define ACCESS_TYPE 13
#define MEMSUFFIX   MMU_MODE13_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 14) */

#if (NB_MMU_MODES >= 15)

#define ACCESS_TYPE 14
#define MEMSUFFIX   MMU_MODE14_SUFFIX
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX
#endif /* (NB_MMU_MODES >= 15) */

//  Adjust sizes of 'tlb_table_n_0' arrays in tcg/additional.{c,h} to
//  NB_MMU_MODES+1 after expanding the number of supported NB_MMU_MODES.
#if (NB_MMU_MODES > 15)
#error "NB_MMU_MODES > 15 is not supported for now"
#endif /* (NB_MMU_MODES > 15) */

/* these access are slower, they must be as rare as possible */
#define ACCESS_TYPE (NB_MMU_MODES)
#define MEMSUFFIX   _data
#define DATA_SIZE   1
#include "softmmu_header.h"

#define DATA_SIZE 2
#include "softmmu_header.h"

#define DATA_SIZE 4
#include "softmmu_header.h"

#define DATA_SIZE 8
#include "softmmu_header.h"
#undef ACCESS_TYPE
#undef MEMSUFFIX

#define ldub(p) ldub_data(p)
#define ldsb(p) ldsb_data(p)
#define lduw(p) lduw_data(p)
#define ldsw(p) ldsw_data(p)
#define ldl(p)  ldl_data(p)
#define ldq(p)  ldq_data(p)

#define ldub_graceful(p, err) ldub_err_data(p, err)
#define ldsb_graceful(p, err) ldsb_err_data(p, err)
#define lduw_graceful(p, err) lduw_err_data(p, err)
#define ldsw_graceful(p, err) ldsw_err_data(p, err)
#define ldl_graceful(p, err)  ldl_err_data(p, err)
#define ldq_graceful(p, err)  ldq_err_data(p, err)

#define stb(p, v) stb_data(p, v)
#define stw(p, v) stw_data(p, v)
#define stl(p, v) stl_data(p, v)
#define stq(p, v) stq_data(p, v)

#define MMUSUFFIX _mmu

#define SHIFT 0
#include "softmmu_template.h"

#define SHIFT 1
#include "softmmu_template.h"

#define SHIFT 2
#include "softmmu_template.h"

#define SHIFT 3
#include "softmmu_template.h"
