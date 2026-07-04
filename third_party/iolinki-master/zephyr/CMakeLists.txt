# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: GPL-3.0-or-later

if(CONFIG_IOLINKI_MASTER)

# The master builds against the iolinki device stack, which provides the shared
# IO-Link frame/CRC helper sources (src/crc.c, src/frame.c) and the public
# <iolinki/*.h> headers. In a west workspace the iolinki repo is a sibling
# Zephyr module, so Zephyr exports its location as ZEPHYR_IOLINKI_MODULE_DIR.
# Fall back to a sibling checkout for non-west builds (ZEPHYR_EXTRA_MODULES).
if(DEFINED ZEPHYR_IOLINKI_MODULE_DIR)
    set(IOLINKI_DEVICE_DIR "${ZEPHYR_IOLINKI_MODULE_DIR}")
else()
    set(IOLINKI_DEVICE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../iolinki")
endif()

if(NOT EXISTS "${IOLINKI_DEVICE_DIR}/src/frame.c")
    message(FATAL_ERROR
        "iolinki device stack not found at '${IOLINKI_DEVICE_DIR}'. Add the "
        "iolinki repository to your west workspace (see the top-level west.yml) "
        "or point ZEPHYR_EXTRA_MODULES at a sibling checkout.")
endif()

zephyr_library()

# Public master API (iolinki_master/*) and the device stack's public headers
# (iolinki/*) are exported globally so the library and the application can both
# include them, e.g. <iolinki_master/master.h> and <iolinki/phy.h>.
zephyr_include_directories(${CMAKE_CURRENT_SOURCE_DIR}/../include)
zephyr_include_directories(${IOLINKI_DEVICE_DIR}/include)

zephyr_library_sources(
    ../src/master_controller.c
    ../src/master_isdu.c
    ../src/master_parameters.c
    ../src/master_port.c
    ../src/master_sio.c
)

# Shared IO-Link frame/CRC helpers from the device stack. When the device
# module is also enabled (CONFIG_IOLINKI=y) it compiles these itself, so build
# them here only when the device library is absent, to avoid duplicate symbols.
if(NOT CONFIG_IOLINKI)
    zephyr_library_sources(
        ${IOLINKI_DEVICE_DIR}/src/crc.c
        ${IOLINKI_DEVICE_DIR}/src/frame.c
    )
endif()

endif() # CONFIG_IOLINKI_MASTER
