#pragma once

#include <stdint.h>
#include "arch_exports_common.h"

uint32_t tlib_get_cpu_model_id(void);
uint32_t tlib_get_it_state(void);
uint32_t tlib_evaluate_condition_code(uint32_t);
uint32_t tlib_get_arm_feature(int32_t feature);

void tlib_set_cpu_model_id(uint32_t value);
void tlib_toggle_fpu(int32_t enabled);
void tlib_set_sev_on_pending(int32_t);
void tlib_set_event_flag(int value);
void tlib_set_number_of_mpu_regions(uint32_t value);
uint32_t tlib_get_number_of_mpu_regions();
void tlib_register_tcm_region(uint32_t address, uint64_t size, uint64_t index);

void tlib_update_pmu_counters(int event_id, uint32_t amount);
void tlib_pmu_set_debug(uint32_t debug);

uint32_t tlib_get_exception_vector_address(void);
void tlib_set_exception_vector_address(uint32_t address);

#ifdef TARGET_PROTO_ARM_M
void tlib_set_sleep_on_exception_exit(int32_t);

void tlib_set_interrupt_vector_base(uint32_t address, bool secure);
uint32_t tlib_get_interrupt_vector_base(bool secure);
void tlib_set_fpu_interrupt_number(int32_t enabled);

void tlib_do_lazy_floating_point_state_preservation(bool createContext);

uint32_t tlib_get_primask(bool secure);
uint32_t tlib_get_faultmask(bool secure);

uint32_t tlib_get_ccr(bool secure);
void tlib_set_ccr(uint32_t value, bool secure);
uint32_t tlib_get_xpsr(void);
uint32_t tlib_get_fault_status(bool secure);
void tlib_set_fault_status(uint32_t value, bool secure);
uint32_t tlib_get_bus_fault_address(void);
void tlib_raise_precise_bus_fault(uint32_t address);
uint32_t tlib_is_locked_up(void);
void tlib_enter_reset_lockup(void);
uint32_t tlib_get_memory_fault_address(bool secure);
uint32_t tlib_get_secure_fault_status();
void tlib_set_secure_fault_status(uint32_t value);
uint32_t tlib_get_secure_fault_address(void);
void tlib_enable_mpu(int32_t enabled);
void tlib_set_mpu_region_base_address(uint32_t value);
void tlib_set_mpu_region_size_and_enable(uint32_t value);
void tlib_set_mpu_region_number(uint32_t value);
uint32_t tlib_is_mpu_enabled();
uint32_t tlib_get_mpu_region_base_address();
uint32_t tlib_get_mpu_region_size_and_enable();
uint32_t tlib_get_mpu_region_number();

uint32_t tlib_get_architecture_version();

void tlib_set_pmsav8_ctrl(uint32_t value, bool secure);
void tlib_set_pmsav8_rnr(uint32_t value, bool secure);
void tlib_set_pmsav8_rbar(uint32_t value, uint32_t region_offset, bool secure);
void tlib_set_pmsav8_rlar(uint32_t value, uint32_t region_offset, bool secure);
void tlib_set_pmsav8_mair(uint32_t index, uint32_t value, bool secure);
uint32_t tlib_get_pmsav8_ctrl(bool secure);
uint32_t tlib_get_pmsav8_rnr(bool secure);
uint32_t tlib_get_pmsav8_rbar(uint32_t region_offset, bool secure);
uint32_t tlib_get_pmsav8_rlar(uint32_t region_offset, bool secure);
uint32_t tlib_get_pmsav8_mair(uint32_t index, bool secure);

void tlib_set_idau_enabled(bool value);
void tlib_set_custom_idau_handler_enabled(bool value);
void tlib_set_number_of_idau_regions(uint32_t value);
void tlib_set_idau_region_base_address_register(uint32_t index, uint32_t value);
void tlib_set_idau_region_limit_address_register(uint32_t index, uint32_t value);
uint32_t tlib_get_idau_enabled();
uint32_t tlib_get_number_of_idau_regions();
uint32_t tlib_get_idau_region_base_address_register(uint32_t index);
uint32_t tlib_get_idau_region_limit_address_register(uint32_t index);

bool tlib_get_process_sp();
void tlib_set_process_sp(bool use_process_sp);
#endif
