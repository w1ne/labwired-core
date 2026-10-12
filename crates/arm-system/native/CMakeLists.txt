cmake_minimum_required(VERSION 3.18)
project(labwired_arm_system C)
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../../../third_party/tlib tlib)
target_sources(tlib PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/bridge.c)
target_include_directories(tlib PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/../../../third_party/tlib/include)
target_link_options(tlib PRIVATE -Wl,-Bsymbolic -Wl,--wrap=free_code_gen_buf)

# Preserve the pinned vendor bytes. Renode's decoder treats both unconditional
# branch-to-self encodings as WFI; Linux busy loops must keep retiring instead.
# Build a derived decoder in OUT_DIR, disabling only these two special cases so
# the existing ordinary ARM/Thumb branch decoder handles the instructions.
set(arm_decoder "${CMAKE_CURRENT_SOURCE_DIR}/../../../third_party/tlib/arch/arm/translate.c")
file(READ "${arm_decoder}" arm_decoder_source)
foreach(opcode IN ITEMS 0xeafffffe 0xe7fe)
  string(REGEX MATCHALL "if[(]insn == ${opcode}[)]" predicate_matches "${arm_decoder_source}")
  list(LENGTH predicate_matches predicate_count)
  if(NOT predicate_count EQUAL 1)
    message(FATAL_ERROR "Pinned ARM branch-self predicate ${opcode}: expected exactly one occurrence")
  endif()
  string(REPLACE "if(insn == ${opcode})"
    "if(0 /* LabWired: branch-self retires; it is not WFI */)"
    arm_decoder_source "${arm_decoder_source}")
endforeach()
set(derived_decoder "${CMAKE_CURRENT_BINARY_DIR}/arm-translate.c")
file(WRITE "${derived_decoder}" "${arm_decoder_source}")
get_target_property(tlib_sources tlib SOURCES)
# GLOB resolves source paths; normalize our parent-relative path for removal.
get_filename_component(arm_decoder_absolute "${arm_decoder}" ABSOLUTE)
list(FIND tlib_sources "${arm_decoder_absolute}" decoder_index)
if(decoder_index EQUAL -1)
  message(FATAL_ERROR "Pinned ARM decoder is absent from tlib source list")
endif()
list(REMOVE_AT tlib_sources ${decoder_index})
list(APPEND tlib_sources "${derived_decoder}")

# code_gen_free releases tbs before growth. If BOTH subsequent mappings fail,
# native fatal containment returns to Rust and Drop disposes the engine again.
# Clear the released pointer so that disposal cannot free it a second time.
set(exec_source "${CMAKE_CURRENT_SOURCE_DIR}/../../../third_party/tlib/exec.c")
file(READ "${exec_source}" exec_contents)
string(REGEX MATCHALL "tlib_free[(]tbs[)]" free_matches "${exec_contents}")
list(LENGTH free_matches free_count)
if(NOT free_count EQUAL 1)
  message(FATAL_ERROR "Pinned JIT block-table release: expected exactly one occurrence")
endif()
string(REPLACE "tlib_free(tbs);" "tlib_free(tbs);\n    tbs = NULL;"
  exec_contents "${exec_contents}")
set(derived_exec "${CMAKE_CURRENT_BINARY_DIR}/exec.c")
file(WRITE "${derived_exec}" "${exec_contents}")
get_filename_component(exec_absolute "${exec_source}" ABSOLUTE)
list(FIND tlib_sources "${exec_absolute}" exec_index)
if(exec_index EQUAL -1)
  message(FATAL_ERROR "Pinned JIT exec source is absent from tlib source list")
endif()
list(REMOVE_AT tlib_sources ${exec_index})
list(APPEND tlib_sources "${derived_exec}")
# The pinned Cortex-A7 declares NEON/VFP4, but omits their VFP3/FP16
# decoder prerequisites and advertises the FPU-only D16 MVFR profile.
# Build a derived helper with the NEON MPE profile (ARM DDI 0462E).
# Match the entire pinned A7 case once; never modify another CPU model or vendor bytes.
set(arm_helper "${CMAKE_CURRENT_SOURCE_DIR}/../../../third_party/tlib/arch/arm/helper.c")
file(READ "${arm_helper}" arm_helper_source)
set(original_a7_case [=[
        case ARM_CPUID_CORTEXA7:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V7SEC);
            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_NEON);
            set_feature(env, ARM_FEATURE_GENERIC_TIMER);
            set_feature(env, ARM_FEATURE_THUMB2EE);
            set_feature(env, ARM_FEATURE_LPAE);
            set_feature(env, ARM_FEATURE_V7MP);
            set_feature(env, ARM_FEATURE_ARM_DIV);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x41023070;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x10110221;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x11000011;
            memcpy(env->cp15.c0_c1, cortexa7_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, cortexa7_cp15_c0_c2, 8 * sizeof(uint32_t));
            env->cp15.c0_cachetype = 0x84448003;
            env->cp15.c0_clid = (1 << 27) | (2 << 24) | (1 << 21) | 3;
            env->cp15.c0_ccsid[0] = 0x7003E01A; /* 8k L1 dcache */
            env->cp15.c0_ccsid[1] = 0x200FE009; /* 8k L1 icache */
            env->cp15.c0_ccsid[2] = 0x701FE03A; /* 128k L2 cache */
            env->cp15.c1_sys = 0x00C50078;
            break;
]=])
string(REPLACE "${original_a7_case}" "" helper_without_a7 "${arm_helper_source}")
string(LENGTH "${arm_helper_source}" helper_length)
string(LENGTH "${helper_without_a7}" helper_without_a7_length)
string(LENGTH "${original_a7_case}" a7_case_length)
math(EXPR matched_a7_length "${helper_length} - ${helper_without_a7_length}")
if(NOT matched_a7_length EQUAL a7_case_length)
  message(FATAL_ERROR "Pinned Cortex-A7 helper profile: expected exactly one original case block")
endif()
set(neon_a7_case "${original_a7_case}")
string(REPLACE "set_feature(env, ARM_FEATURE_VFP4);"
  "set_feature(env, ARM_FEATURE_VFP3);\n            set_feature(env, ARM_FEATURE_VFP4);\n            set_feature(env, ARM_FEATURE_VFP_FP16);"
  neon_a7_case "${neon_a7_case}")
string(REPLACE "0x10110221" "0x10110222" neon_a7_case "${neon_a7_case}")
string(REPLACE "0x11000011" "0x11111111" neon_a7_case "${neon_a7_case}")
string(REPLACE "${original_a7_case}" "${neon_a7_case}"
  arm_helper_source "${arm_helper_source}")
set(derived_helper "${CMAKE_CURRENT_BINARY_DIR}/arm-helper.c")
file(WRITE "${derived_helper}" "${arm_helper_source}")
get_filename_component(arm_helper_absolute "${arm_helper}" ABSOLUTE)
list(FIND tlib_sources "${arm_helper_absolute}" helper_index)
if(helper_index EQUAL -1)
  message(FATAL_ERROR "Pinned ARM helper is absent from tlib source list")
endif()
list(REMOVE_AT tlib_sources ${helper_index})
list(APPEND tlib_sources "${derived_helper}")
set_property(TARGET tlib PROPERTY SOURCES "${tlib_sources}")
