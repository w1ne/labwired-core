cmake_minimum_required(VERSION 3.10)

project(iolinki_master VERSION 0.2.0 LANGUAGES C)

set(CMAKE_C_STANDARD 99)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wall -Wextra")

# Code coverage instrumentation (used by the release workflow).
option(ENABLE_COVERAGE "Enable code coverage instrumentation" OFF)
if(ENABLE_COVERAGE)
    if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
        set(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} --coverage -fprofile-arcs -ftest-coverage")
        set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} --coverage")
    endif()
endif()

set(IOLINKI_DEVICE_DIR
    "${CMAKE_CURRENT_SOURCE_DIR}/../iolinki"
    CACHE PATH "Path to the local iolinki device repository")

if(NOT EXISTS "${IOLINKI_DEVICE_DIR}/src/frame.c")
    message(FATAL_ERROR
        "iolinki device stack not found at '${IOLINKI_DEVICE_DIR}'. "
        "Clone it (git clone -b develop git@github.com:w1ne/iolinki.git) "
        "next to this repository, or pass -DIOLINKI_DEVICE_DIR=/path/to/iolinki.")
endif()

option(BUILD_TESTING "Build unit tests" ON)
option(BUILD_EXAMPLES "Build runnable examples" ON)

add_library(iolinki_frame_helpers STATIC
    "${IOLINKI_DEVICE_DIR}/src/crc.c"
    "${IOLINKI_DEVICE_DIR}/src/frame.c"
)
target_include_directories(iolinki_frame_helpers PUBLIC "${IOLINKI_DEVICE_DIR}/include")

add_library(iolinki_master STATIC
    src/master_controller.c
    src/master_isdu.c
    src/master_parameters.c
    src/master_port.c
    src/master_sio.c
)
target_include_directories(iolinki_master PUBLIC include "${IOLINKI_DEVICE_DIR}/include")
target_link_libraries(iolinki_master PUBLIC iolinki_frame_helpers)

if(BUILD_EXAMPLES)
    add_executable(master_loopback_demo examples/master_loopback_demo.c)
    target_link_libraries(master_loopback_demo iolinki_master)

    add_executable(master_4port_controller_demo examples/master_4port_controller_demo.c)
    target_link_libraries(master_4port_controller_demo iolinki_master)
endif()

# Optional Doxygen API documentation target (`cmake --build build --target doc`).
option(IOLINK_MASTER_ENABLE_DOCS "Enable Doxygen documentation target" OFF)
if(IOLINK_MASTER_ENABLE_DOCS)
    find_package(Doxygen)
    if(DOXYGEN_FOUND)
        add_custom_target(doc
            COMMAND ${DOXYGEN_EXECUTABLE} ${CMAKE_CURRENT_SOURCE_DIR}/Doxyfile
            WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
            COMMENT "Generating API documentation with Doxygen"
            VERBATIM)
    else()
        message(WARNING "Doxygen not found; documentation target disabled.")
    endif()
endif()

if(BUILD_TESTING)
    enable_testing()
    add_subdirectory(tests)
    if(BUILD_EXAMPLES)
        add_test(NAME master_loopback_demo COMMAND master_loopback_demo)
        add_test(NAME master_4port_controller_demo COMMAND master_4port_controller_demo)
    endif()
endif()
