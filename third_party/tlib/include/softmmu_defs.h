#pragma once
/* Common softmmu declarations.
 * For implementations refer to softmmu_template.h
 */

/* When accessing memory from a helper function, use the __inner versions. The non-__inner versions are intended to be called only
 * from generated code, because they retrieve the calling TB's return address directly from the call stack to identify the current
 * translation block. When called from a C helper instead, the return address points into the helper, not into generated code,
 * causing TB lookup to fail or return a wrong result. In spite of being declared with always_inline attribute the non-__inner
 * functions won't inline into helpers.
 */

uint8_t REGPARM __ldb_mmu(target_ulong addr, int mmu_idx);
uint8_t REGPARM __ldb_err_mmu(target_ulong addr, int mmu_idx, int *err);
uint8_t REGPARM __inner_ldb_err_mmu(target_ulong addr, int mmu_idx, int *err, void *retaddr);
void REGPARM __stb_mmu(target_ulong addr, uint8_t val, int mmu_idx);
void REGPARM __inner_stb_mmu(target_ulong addr, uint8_t val, int mmu_idx, void *retaddr);
uint16_t REGPARM __ldw_mmu(target_ulong addr, int mmu_idx);
uint16_t REGPARM __ldw_err_mmu(target_ulong addr, int mmu_idx, int *err);
uint16_t REGPARM __inner_ldw_err_mmu(target_ulong addr, int mmu_idx, int *err, void *retaddr);
void REGPARM __stw_mmu(target_ulong addr, uint16_t val, int mmu_idx);
void REGPARM __inner_stw_mmu(target_ulong addr, uint16_t val, int mmu_idx, void *retaddr);
uint32_t REGPARM __ldl_mmu(target_ulong addr, int mmu_idx);
uint32_t REGPARM __ldl_err_mmu(target_ulong addr, int mmu_idx, int *err);
uint32_t REGPARM __inner_ldl_err_mmu(target_ulong addr, int mmu_idx, int *err, void *retaddr);
void REGPARM __stl_mmu(target_ulong addr, uint32_t val, int mmu_idx);
void REGPARM __inner_stl_mmu(target_ulong addr, uint32_t val, int mmu_idx, void *retaddr);
uint64_t REGPARM __ldq_mmu(target_ulong addr, int mmu_idx);
uint64_t REGPARM __ldq_err_mmu(target_ulong addr, int mmu_idx, int *err);
uint64_t REGPARM __inner_ldq_err_mmu(target_ulong addr, int mmu_idx, int *err, void *retaddr);
void REGPARM __stq_mmu(target_ulong addr, uint64_t val, int mmu_idx);
void REGPARM __inner_stq_mmu(target_ulong addr, uint64_t val, int mmu_idx, void *retaddr);

uint8_t REGPARM __ldb_cmmu(target_ulong addr, int mmu_idx);
uint8_t REGPARM __ldb_err_cmmu(target_ulong addr, int mmu_idx, int *err);
uint8_t REGPARM __inner_ldb_err_cmmu(target_ulong addr, int mmu_idx, int *err, void *retaddr);
void REGPARM __stb_cmmu(target_ulong addr, uint8_t val, int mmu_idx);
void REGPARM __inner_stb_cmmu(target_ulong addr, uint8_t val, int mmu_idx, void *retaddr);
uint16_t REGPARM __ldw_cmmu(target_ulong addr, int mmu_idx);
uint16_t REGPARM __ldw_err_cmmu(target_ulong addr, int mmu_idx, int *err);
uint16_t REGPARM __inner_ldw_err_cmmu(target_ulong addr, int mmu_idx, int *err, void *retaddr);
void REGPARM __stw_cmmu(target_ulong addr, uint16_t val, int mmu_idx);
void REGPARM __inner_stw_cmmu(target_ulong addr, uint16_t val, int mmu_idx, void *retaddr);
uint32_t REGPARM __ldl_cmmu(target_ulong addr, int mmu_idx);
uint32_t REGPARM __ldl_err_cmmu(target_ulong addr, int mmu_idx, int *err);
uint32_t REGPARM __inner_ldl_err_cmmu(target_ulong addr, int mmu_idx, int *err, void *retaddr);
void REGPARM __stl_cmmu(target_ulong addr, uint32_t val, int mmu_idx);
void REGPARM __inner_stl_cmmu(target_ulong addr, uint32_t val, int mmu_idx, void *retaddr);
uint64_t REGPARM __ldq_cmmu(target_ulong addr, int mmu_idx);
uint64_t REGPARM __ldq_err_cmmu(target_ulong addr, int mmu_idx, int *err);
uint64_t REGPARM __inner_ldq_err_cmmu(target_ulong addr, int mmu_idx, int *err, void *retaddr);
void REGPARM __stq_cmmu(target_ulong addr, uint64_t val, int mmu_idx);
void REGPARM __inner_stq_cmmu(target_ulong addr, uint64_t val, int mmu_idx, void *retaddr);
