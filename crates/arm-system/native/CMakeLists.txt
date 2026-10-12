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
set_property(TARGET tlib PROPERTY SOURCES "${tlib_sources}")
